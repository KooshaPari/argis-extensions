//! `argis-monitor` binary entry point.

use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use argis_monitor::{exporter, Config, Monitor, StateStore};

#[derive(Debug, Parser)]
#[command(name = "argis-monitor", version, about = "Observable Integration substrate for bifrost-extensions (Tenet 4).")]
struct Cli {
    /// Path to a YAML config file. CLI flags and env (ARGIS_MONITOR_*) override.
    #[arg(long, global = true, env = "ARGIS_MONITOR_CONFIG")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Start the poll loop + exporter. Blocks until SIGINT/SIGTERM.
    Start {
        /// Single-target shortcut. Equivalent to --targets gateway=<url>.
        #[arg(long, env = "ARGIS_MONITOR_TARGET")]
        target: Option<String>,
        /// Multi-target form: NAME=URL repeated, e.g. --targets openai=http://a --targets anthropic=http://b
        #[arg(long = "targets", value_name = "NAME=URL", env = "ARGIS_MONITOR_TARGETS")]
        targets: Vec<String>,
        #[arg(long, env = "ARGIS_MONITOR_POLL_INTERVAL")]
        poll_interval_secs: Option<u64>,
        #[arg(long, env = "ARGIS_MONITOR_EXPORTER_ADDR", default_value = "0.0.0.0:9090")]
        exporter_addr: String,
    },
    /// Run exactly one poll (for smoke tests + cron).
    Once {
        #[arg(long, env = "ARGIS_MONITOR_TARGET")]
        target: Option<String>,
        #[arg(long = "targets", value_name = "NAME=URL")]
        targets: Vec<String>,
    },
    /// Validate a config file: parse it and print the resolved struct.
    ValidateConfig {
        #[arg(long)]
        config: PathBuf,
    },
    /// Prune alert_history + alert_state rows older than the threshold.
    Prune {
        #[arg(long, global = true, env = "ARGIS_MONITOR_CONFIG")]
        config: Option<PathBuf>,
        #[arg(long, env = "ARGIS_MONITOR_DATA_DIR")]
        data_dir: Option<PathBuf>,
        #[arg(long)]
        older_than_days: u64,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        dry_run: bool,
    },
}

fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(run(cli))
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Cmd::Start { target, targets, poll_interval_secs, exporter_addr } => {
            let mut cfg = load_config(cli.config.as_deref())?;
            apply_cli_targets(&mut cfg, target, &targets);
            if let Some(s) = poll_interval_secs {
                cfg.poll_interval = Duration::from_secs(s);
            }
            // CLI exporter_addr overrides only if non-default
            if exporter_addr != "0.0.0.0:9090" {
                cfg.exporter_addr = exporter_addr;
            }
            run_monitor(cfg).await
        }
        Cmd::Once { target, targets } => {
            let mut cfg = load_config(cli.config.as_deref())?;
            apply_cli_targets(&mut cfg, target, &targets);
            let monitor = Monitor::new(cfg)?;
            let outcome = monitor.poll_once().await?;
            println!("{}", serde_json::to_string_pretty(&outcome)?);
            Ok(())
        }
        Cmd::ValidateConfig { config } => {
            let cfg = load_config(Some(&config))?;
            println!("{}", serde_json::to_string_pretty(&cfg)?);
            Ok(())
        }
        Cmd::Prune { config, data_dir, older_than_days, dry_run } => {
            run_prune(config, data_dir, older_than_days, dry_run)
        }
    }
}

async fn run_monitor(cfg: Config) -> anyhow::Result<()> {
    let monitor = Monitor::new(cfg.clone())?;
    let registry = monitor.registry();
    let _exp = exporter::serve(&cfg.exporter_addr, registry.clone()).await?;
    if let Some(push_url) = cfg.push_url.clone() {
        let job = cfg.push_job.clone()
            .or_else(|| std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "argis-monitor".to_string());
        let instance = cfg.push_instance.clone()
            .unwrap_or_else(|| format!("host-{}", std::process::id()));
        let interval = std::time::Duration::from_secs(cfg.push_interval_secs);
        tokio::spawn(async move {
            argis_monitor::run_pusher(push_url, registry, interval, job, instance).await;
        });
    }
    monitor.run().await
}

fn load_config(path: Option<&std::path::Path>) -> anyhow::Result<Config> {
    if let Some(p) = path {
        let bytes = std::fs::read(p)?;
        let cfg: Config = serde_yaml::from_slice(&bytes)
            .map_err(|e| anyhow::anyhow!("invalid yaml in {}: {e}", p.display()))?;
        Ok(cfg)
    } else {
        Ok(Config::default())
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}


/// Apply CLI --target / --targets NAME=URL onto a Config. Only used if the
/// config came from the CLI (no config file), so it never overrides a
/// YAML-defined target list silently.
fn apply_cli_targets(cfg: &mut Config, target: Option<String>, targets: &[String]) {
    if let Some(t) = target {
        cfg.targets.clear();
        cfg.targets.push(argis_monitor::Target::new("gateway", t));
    }
    for t in targets {
        if let Some((name, url)) = t.split_once('=') {
            cfg.targets.push(argis_monitor::Target::new(name, url));
        } else {
            tracing::warn!(target = %t, "ignoring malformed NAME=URL target");
        }
    }
}

fn run_prune(config_path: Option<PathBuf>, data_dir: Option<PathBuf>, days: u64, dry_run: bool) -> anyhow::Result<()> {
    let dir = if let Some(d) = data_dir {
        d
    } else if let Some(c) = config_path {
        let cfg = load_config(Some(&c))?;
        cfg.data_dir.unwrap_or_else(|| std::path::PathBuf::from("./data"))
    } else {
        std::path::PathBuf::from("./data")
    };
    let db = dir.join("alert_state.sqlite");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    let threshold = now.saturating_sub(days * 86_400);
    if !db.exists() {
        println!("No state store at {}; nothing to prune.", db.display());
        return Ok(());
    }
    let mut store = StateStore::open(&db)?;
    let would_delete = store.count_history_before(threshold)?;
    let chrono_date = chrono::DateTime::<chrono::Utc>::from_timestamp(threshold as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| threshold.to_string());
    if dry_run {
        println!("Would delete {} rows from alert_history (rows older than {})", would_delete, chrono_date);
        return Ok(());
    }
    let report = store.prune(threshold)?;
    println!("Deleted {} rows from alert_history, {} rows from alert_state (rows older than {})",
             report.alert_history_deleted, report.alert_state_deleted, chrono_date);
    Ok(())
}
