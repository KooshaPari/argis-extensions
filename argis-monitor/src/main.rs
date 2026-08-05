//! `argis-monitor` binary entry point.

use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use argis_monitor::{exporter, Config, Monitor};

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
        #[arg(long, env = "ARGIS_MONITOR_POLL_INTERVAL", default_value = "15")]
        poll_interval_secs: u64,
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
            cfg.poll_interval = Duration::from_secs(poll_interval_secs);
            cfg.exporter_addr = exporter_addr;
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
    }
}

async fn run_monitor(cfg: Config) -> anyhow::Result<()> {
    let monitor = Monitor::new(cfg.clone())?;
    let _handle = exporter::serve(&cfg.exporter_addr, monitor.registry()).await?;
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
    if target.is_some() || !targets.is_empty() {
        cfg.targets.clear();
    }
    if let Some(t) = target {
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
