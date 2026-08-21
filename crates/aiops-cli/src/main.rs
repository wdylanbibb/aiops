mod args;

use std::io::Write;

use aiops_core::observations::ObservationBundle;
use aiops_diagnostics::DiagnosticEngine;
use aiops_k8s::{
    client::KubernetesClient,
    collector::{CollectionOptions, KubernetesCollector},
};
use anyhow::Context;
use clap::Parser;

use crate::args::{CollectArgs, Command, DiagnoseArgs, PodArgs, ResourceCommand};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = args::Cli::parse();

    match cli.command {
        Command::Collect(CollectArgs {
            resource: ResourceCommand::Pod(args),
        }) => collect_pod(args).await?,
        Command::Diagnose(DiagnoseArgs {
            resource: ResourceCommand::Pod(args),
        }) => diagnose_pod(args).await?,
    }

    Ok(())
}

async fn collect_pod(args: PodArgs) -> anyhow::Result<()> {
    let bundle = collect_pod_observations(&args).await?;
    write_json(&bundle).context("failed to write observation bundle")
}

async fn diagnose_pod(args: PodArgs) -> anyhow::Result<()> {
    let bundle = collect_pod_observations(&args).await?;
    let report = DiagnosticEngine::default_rules().diagnose(&bundle);
    write_json(&report).context("failed to write diagnosis report")
}

async fn collect_pod_observations(args: &PodArgs) -> anyhow::Result<ObservationBundle> {
    let options = collection_options(args)?;
    let client = KubernetesClient::infer()
        .await
        .context("failed to connect to Kubernetes")?;
    let collector = KubernetesCollector::new(client, options);
    collector
        .collect_pod(&args.namespace, &args.name)
        .await
        .with_context(|| format!("failed to collect pod {}/{}", args.namespace, args.name))
}

fn write_json(value: &impl serde::Serialize) -> anyhow::Result<()> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, value)?;
    writeln!(output)?;

    Ok(())
}

fn collection_options(args: &PodArgs) -> anyhow::Result<CollectionOptions> {
    Ok(CollectionOptions {
        lookback: chrono::Duration::from_std(args.lookback)
            .context("lookback duration is too large")?,
        tail_lines: args
            .tail_lines
            .try_into()
            .context("tail line count is too large")?,
        include_logs: !args.no_logs,
        include_previous_logs: !args.no_previous_logs,
        include_events: !args.no_events,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn pod_args() -> PodArgs {
        PodArgs {
            name: "api-0".to_owned(),
            namespace: "production".to_owned(),
            lookback: Duration::from_secs(30 * 60),
            tail_lines: 250,
            no_logs: true,
            no_previous_logs: true,
            no_events: true,
        }
    }

    #[test]
    fn maps_pod_arguments_to_collection_options() {
        let options = collection_options(&pod_args()).unwrap();

        assert_eq!(options.lookback, chrono::Duration::minutes(30));
        assert_eq!(options.tail_lines, 250);
        assert!(!options.include_logs);
        assert!(!options.include_previous_logs);
        assert!(!options.include_events);
    }

    #[test]
    fn rejects_tail_count_larger_than_collector_supports() {
        let mut args = pod_args();
        args.tail_lines = u64::MAX;

        assert!(collection_options(&args).is_err());
    }
}
