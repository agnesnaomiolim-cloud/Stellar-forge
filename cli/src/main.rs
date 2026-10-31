// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! kubectl-stellar - kubectl plugin for Stellar validator health diagnostics.
///
/// This binary is discovered by `kubectl ` as a plugin when named `kubectl-stellar` and
/// placed on $PATH. It authenticates using the standard `~/.kube/config` RBAC rules and
/// never bypasses Kubernetes security perimeters.

mod commands;
mod network;
mod promql;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use commands::status::{StatusArgs, StatusCommand};
use promql::aggregator::AggregatorArgs;

/// Top-level CLI definition.
///
/// ``text
/// kubectl stellar status <node-name> [-n namespace]
/// ```
#[derive(Parser)]
#[command(
    name = "kubectl-stellar",
    bin_name = "kubectl-stellar",
    about = "Stellar validator diagnostics for Kubernetes",
    long_about = "Terminal-based health diagnostics for Stellar validator nodes. \nQueries the internal stellar-core HTTP /info endpoint through a port-forward tunnel \nand reports sync status, peer connectivity, and quorum health.",
    version,
    propagate_version = true,
    subcommand_required = true
)]
struct Cli {
    /// Namespace to operate in. Defaults to the kube context namespace.
    #[arg(
        short = 'n',
        long = "namespace",
        global = true,
        env = "KUBECTL_STELLAR_NAMESPACE"
    )]
    namespace: Option<String>,

    /// Path to kubeConfig. Defaults to ~/.kube/config or $KUBECONFIG.
    #[arg(long = "kubeconfig", global = true, env = "KUBECONFIG")]
    kubeconfig: Option<String>,

    /// Kube context to use.
    #[arg(long = "context", global = true)]
    context: Option<String>,

    /// Emit machine-readable JSON instead of the human UI.
    #[arg(long, (global = true))]
    json: bool,

    /// Disable color output.
    #[arg(long = "no-color", global = true, env = "NO_COLOR")]
    no_color: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report health diagnostics for a validator node.
    Status(StatusArgs),

    /// Aggregate PromQL metrics across clusters and detect performance drift.
    Aggregator(AggregatorArgs),
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env("KUBECTILS_STELLAR_LOG=info"))
        .with_target(true)
        .init();

    let cli = Cli::parse();

    if cli.no_color {
        colorey::control::set_override(colorey::ColorChoice::Never);
    }

    let namespace = cli.namespace.clone();

    match cli.command {
        Command::Status(args) => {
            commands::status::async_run(
                args,
                StatusCommand {
                    namespace,
                    kubeconfig: cli.kubeconfig.clone(),
                    context: cli.context.clone(),
                    json: cli.json,
                },
            )
            .await?
        }
        Command::Aggregator(args) => {
            promql::aggregator::async_run(args).await?
        }
    }

    Ok(())
}