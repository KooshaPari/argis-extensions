//! `argis-monitor` binary entry point.

use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use argis_monitor::{exporter, Config, Monitor};

#[derive(Debug, Parser)]
#[command(
    name = "argis-monitor",
    version,
    about = "Observable Integration substrate for bifrost-extensions (Tenet 4)."
)]
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
        #[arg(long, env = "ARGIS_MONITOR_TARGET")]
        target: Option<String>,
        #[arg(long, env = "ARGIS_MONITOR_POLL_INTERVAL")]
        poll_interval_secs: Option<u64>,
        #[arg(long, env = "ARGIS_MONITOR_EXPORTER_ADDR")]
        exporter_addr: Option<String>,
    },
    /// Run exactly one poll (for smoke tests + cron).
    Once {
        #[arg(long, env = "ARGIS_MONITOR_TARGET")]
        target: Option<String>,
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
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run(cli))
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Cmd::Start {
            target,
            poll_interval_secs,
            exporter_addr,
        } => {
            let mut cfg = load_config(cli.config.as_deref())?;
            if let Some(t) = target {
                cfg.target = t;
            }
            if let Some(secs) = poll_interval_secs {
                cfg.poll_interval = Duration::from_secs(secs);
            }
            if let Some(addr) = exporter_addr {
                cfg.exporter_addr = addr;
            }
            run_monitor(cfg).await
        }
        Cmd::Once { target } => {
            let mut cfg = load_config(cli.config.as_deref())?;
            if let Some(t) = target {
                cfg.target = t;
            }
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
