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

use crate::args::{CollectArgs, Command, DiagnoseArgs, ResourceArgs, ResourceCommand};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = args::Cli::parse();

    match cli.command {
        Command::Collect(CollectArgs { resource }) => collect(resource).await?,
        Command::Diagnose(DiagnoseArgs { resource }) => diagnose(resource).await?,
    }

    Ok(())
}

async fn collect(resource: ResourceCommand) -> anyhow::Result<()> {
    let bundle = collect_observations(&resource).await?;
    write_json(&bundle).context("failed to write observation bundle")
}

async fn diagnose(resource: ResourceCommand) -> anyhow::Result<()> {
    let bundle = collect_observations(&resource).await?;
    let incident = DiagnosticEngine::default_rules().diagnose(bundle);
    write_json(&incident).context("failed to write incident")
}

async fn collect_observations(resource: &ResourceCommand) -> anyhow::Result<ObservationBundle> {
    let (kind, args) = resource_parts(resource);
    let options = collection_options(args)?;
    let client = KubernetesClient::infer()
        .await
        .context("failed to connect to Kubernetes")?;
    let collector = KubernetesCollector::new(client, options);

    let observations = match resource {
        ResourceCommand::Pod(_) => collector.collect_pod(&args.namespace, &args.name).await,
        ResourceCommand::ReplicaSet(_) => {
            collector
                .collect_replica_set(&args.namespace, &args.name)
                .await
        }
        ResourceCommand::Deployment(_) => {
            collector
                .collect_deployment(&args.namespace, &args.name)
                .await
        }
    };

    observations
        .with_context(|| format!("failed to collect {kind} {}/{}", args.namespace, args.name))
}

fn resource_parts(resource: &ResourceCommand) -> (&'static str, &ResourceArgs) {
    match resource {
        ResourceCommand::Pod(args) => ("pod", args),
        ResourceCommand::ReplicaSet(args) => ("replica set", args),
        ResourceCommand::Deployment(args) => ("deployment", args),
    }
}

fn write_json(value: &impl serde::Serialize) -> anyhow::Result<()> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, value)?;
    writeln!(output)?;

    Ok(())
}

fn collection_options(args: &ResourceArgs) -> anyhow::Result<CollectionOptions> {
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

    fn resource_args() -> ResourceArgs {
        ResourceArgs {
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
        let options = collection_options(&resource_args()).unwrap();

        assert_eq!(options.lookback, chrono::Duration::minutes(30));
        assert_eq!(options.tail_lines, 250);
        assert!(!options.include_logs);
        assert!(!options.include_previous_logs);
        assert!(!options.include_events);
    }

    #[test]
    fn rejects_tail_count_larger_than_collector_supports() {
        let mut args = resource_args();
        args.tail_lines = u64::MAX;

        assert!(collection_options(&args).is_err());
    }

    #[test]
    fn identifies_each_collector_target() {
        for (resource, expected) in [
            (ResourceCommand::Pod(resource_args()), "pod"),
            (ResourceCommand::ReplicaSet(resource_args()), "replica set"),
            (ResourceCommand::Deployment(resource_args()), "deployment"),
        ] {
            let (kind, args) = resource_parts(&resource);
            assert_eq!(kind, expected);
            assert_eq!(args.name, "api-0");
        }
    }
}
