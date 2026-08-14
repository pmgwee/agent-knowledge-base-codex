use std::path::PathBuf;

use anyhow::{Context, Result};
use brain_cli::{
    AgentSourceOptions, BenchmarkArtifacts, BenchmarkPreflightOptions, BenchmarkProfile,
    RegisterOptions, RetrievalBenchmarkOptions, RetrievalSplit, ServiceInstallOptions,
    SessionFilter, SessionStatusOptions, TaskCommands, benchmark_corpus,
    build_report_from_artifacts, configure_codegraph, configure_llm_wiki, disable_provider,
    evaluate_retrieval_cases, evaluate_retrieval_fixture_cases, execute_benchmark_run,
    export_grading_bundle, import_grades, index_codegraph, install_claude_hooks,
    install_claude_mcp, install_codex_hooks, install_windows_service, latest_summary,
    preflight_benchmark, preview_benchmark_run, provider_status, read_dashboard, read_diagnostics,
    read_gold_cases, read_hermes_status, read_session_status, read_status, rebuild_basic_memory,
    rebuild_markdown, register_project_with_sources, remove_provider, run_retrieval_benchmark,
    run_retrieval_fixture_benchmark, sha256_file, source_fingerprint, start_windows_service,
    stop_windows_service, uninstall_claude_hooks, uninstall_claude_mcp, uninstall_codex_hooks,
    uninstall_windows_service, verify_projections, windows_service_status,
};
use brain_coordination::{ClaimKind, PathClaimInput, SessionIdentity};
use brain_domain::{BrainConfig, Harness, ProjectId, ProjectRegistry};
use brain_service::{
    BrainCheckpointRequest, BrainClaimRequest, BrainClaimsRequest, BrainLeaseAcquireRequest,
    BrainLeaseGenerationRequest, BrainLeaseHandoffRequest, BrainLeasesRequest,
    BrainPreflightRequest, BrainQueryService, BrainReleaseClaimRequest, BrainSearchRequest,
    BrainTimelineRequest, ServiceLaunchConfig, SourceSelector, TimelineWindow,
};
use brain_store::{BackupManager, EventLedger, RetentionPolicy, UpgradeManager};
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "brain", about = "Cross-agent secondary brain operator CLI")]
struct Cli {
    #[arg(long, global = true)]
    brain_home: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Register {
        path: PathBuf,
        #[arg(long)]
        claude_projects_root: Option<PathBuf>,
        #[arg(long)]
        no_discover_claude: bool,
        #[arg(long)]
        codex_sessions_root: Option<PathBuf>,
        #[arg(long)]
        no_discover_codex: bool,
        #[arg(long)]
        hermes_db: Option<PathBuf>,
        #[arg(long)]
        no_hermes: bool,
    },
    Status {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        harness: Option<StatusHarness>,
        #[arg(long)]
        hermes_db: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Sessions {
        #[command(subcommand)]
        action: SessionsCommand,
    },
    InstallHooks {
        harness: HookHarness,
        #[arg(long)]
        settings: Option<PathBuf>,
        #[arg(long)]
        hook_executable: Option<PathBuf>,
    },
    InstallMcp {
        #[arg(long, value_enum)]
        harness: McpHarness,
        #[arg(long)]
        claude_executable: Option<PathBuf>,
    },
    UninstallHooks {
        harness: HookHarness,
        #[arg(long)]
        settings: Option<PathBuf>,
        #[arg(long)]
        hook_executable: Option<PathBuf>,
    },
    UninstallMcp {
        #[arg(long, value_enum)]
        harness: McpHarness,
        #[arg(long)]
        claude_executable: Option<PathBuf>,
    },
    Query {
        #[arg(long)]
        project: String,
        text: String,
        #[arg(long)]
        as_of: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
        /// Re-rank the head of the results with the cross-encoder. Needs the checkpoint under
        /// `models/ms-marco-MiniLM-L6-v2`, and costs roughly 1.5 s.
        #[arg(long)]
        rerank: bool,
        /// Expand the query with the corpus's own vocabulary before ranking. Costs a second
        /// retrieval pass and cannot lose a result the plain query found.
        #[arg(long)]
        expand: bool,
    },
    /// File a conclusion back into the brain, citing the events it rests on.
    Remember {
        #[arg(long)]
        project: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        content: String,
        /// One or more `event:<uuid>` this conclusion rests on.
        ///
        /// **Omit it.** Nobody can know an event UUID by hand, which is why this command shipped
        /// in June and had never been used once across 13,493 memories. Left empty, the citations
        /// are derived by running the claim's own text through the same fused retrieval the rest of
        /// the brain uses, and the chosen turns are printed so you can check them.
        #[arg(long = "evidence")]
        evidence: Vec<String>,
        /// Scope derived citations to one session. Defaults to the whole project.
        #[arg(long)]
        session: Option<String>,
        /// Memory ids this replaces. Their current versions are superseded, never deleted.
        #[arg(long = "supersedes")]
        supersedes: Vec<String>,
        #[arg(long, default_value = "fact")]
        kind: String,
    },
    /// Health-check a project's memories: contradictions, unrefreshed claims, islands.
    Lint {
        #[arg(long)]
        project: String,
        /// Re-date claims the provider dated at the Unix epoch, from the evidence they cite.
        /// A claim rests on all its evidence, so it cannot predate the last piece — this is
        /// arithmetic, not a judgement. Claims whose citations resolve to nothing are left alone.
        #[arg(long)]
        repair_dates: bool,
        #[arg(long)]
        json: bool,
    },
    /// A derived health reading of the brain: what needs a decision, and what merely changed.
    /// `--write` appends it to the project's vault log so a scheduled run leaves a trail.
    Digest {
        /// Omit to cover every registered project — what the scheduled task does.
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        write: bool,
        #[arg(long)]
        json: bool,
    },
    /// Write the paragraph at the top of each subject page.
    ///
    /// Without `--generate` this only reports which subjects have prose describing their current
    /// memory set and which do not — no provider call, nothing written.
    Synthesize {
        #[arg(long)]
        project: String,
        /// Draft, validate and store prose. Nothing that fails validation is written.
        #[arg(long)]
        generate: bool,
        /// Subjects per run. Each is one provider call.
        #[arg(long, default_value_t = 5)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// The consolidation queue, and the one action a stuck queue needs.
    ///
    /// `brain digest` reports a dead-letter count and nothing could act on it — three jobs sat dead
    /// for three days because the only way to retry one was to edit SQLite by hand.
    Jobs {
        #[arg(long)]
        project: String,
        /// Return every dead-lettered job to the queue with its attempt count cleared.
        ///
        /// Manual on purpose. The five-attempt ceiling exists so a job that can never succeed stops
        /// spending a provider call; re-arming it on a timer would undo that. Asking for this is a
        /// human asserting something changed that the queue cannot observe — quota returned, or the
        /// bug it kept hitting is fixed.
        #[arg(long)]
        retry_dead: bool,
        #[arg(long)]
        json: bool,
    },
    /// Replay one captured session as discrete events, or list sessions when none is named.
    Replay {
        #[arg(long)]
        project: String,
        #[arg(long)]
        session: Option<String>,
        /// One event in full, by id — what expanding an `event:<uuid>` citation fetches.
        #[arg(long)]
        event: Option<uuid::Uuid>,
        /// Turns per page. The default is a page, not a session: the largest session here is
        /// 15,969 events and serialises to 100 MB.
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long)]
        json: bool,
    },
    /// Explain why a query returned what it did — per channel, with the fused position beside it.
    Explain {
        #[arg(long)]
        project: String,
        text: String,
        #[arg(long, default_value = "10")]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Propose a resolution for each contradiction `brain lint` finds — derived from authority,
    /// recency and evidence weight, never from a model. Reports rather than applies.
    Reconcile {
        #[arg(long)]
        project: String,
        /// Fold every decided proposal by supersession. Nothing is deleted; the losing versions
        /// keep their evidence and stay reachable. Undecidable contradictions are left alone.
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        json: bool,
    },
    /// Show older claims a newer observation on the same subject appears to complete.
    ///
    /// Karpathy's Ingest operation — "a single source might touch 10–15 wiki pages" — is the one
    /// this system does not perform. Detection is derived; the rewrite is a judgement and stays
    /// yours. Reports and stops.
    /// Rule on memories consolidation wrote but nothing has read yet — A9's ingest checkpoint.
    ///
    /// Only populated for kinds listed in the service config's `review.gated_kinds`. Gated
    /// memories land as `proposed`, which keeps them out of the orientation, out of `search` and
    /// out of the projection until approved.
    Review {
        #[arg(long)]
        project: String,
        /// Make this memory current. It becomes visible to everything that reads the brain.
        #[arg(long)]
        approve: Option<uuid::Uuid>,
        /// Mark this memory invalid. It is kept, not deleted — the ledger records that someone
        /// looked and said no.
        #[arg(long)]
        reject: Option<uuid::Uuid>,
        #[arg(long)]
        json: bool,
    },
    Revise {
        #[arg(long)]
        project: String,
        /// Ask the configured provider to draft the merged claim for each candidate. Prints the
        /// proposals and writes nothing.
        #[arg(long)]
        propose: bool,
        /// Write the proposals that pass every rule, superseding both sides. Requires --propose.
        ///
        /// Approval here is per *run*, not per pair — agreeing with eleven of thirteen still writes
        /// all thirteen. Prefer --review-sheet.
        #[arg(long)]
        apply: bool,
        /// Write the proposals to a JSON sheet for per-pair human approval, instead of applying
        /// them. Requires --propose. Mark each item's `decision` as approve or reject, then pass
        /// the file back with --apply-reviewed.
        #[arg(long)]
        review_sheet: Option<std::path::PathBuf>,
        /// Apply only the items a human marked `approve` in a sheet, using the text they read.
        /// Calls no provider, so nothing is re-drafted between review and write.
        #[arg(long)]
        apply_reviewed: Option<std::path::PathBuf>,
        /// How many candidates to draft in one run. Small on purpose — these cost a provider call
        /// each, and thirteen proposals is more than anyone reviews carefully in one sitting.
        #[arg(long, default_value = "5")]
        limit: usize,
        /// Include pairs written the same day. Off by default: 441 of 454 on this corpus were
        /// consolidation windows overlapping, which is the fold's business, not a revision.
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// Show which memories eviction would retire — and refuse to retire them until access has
    /// been counted long enough for "never retrieved" to mean anything.
    Evict {
        #[arg(long)]
        project: String,
        /// Actually retire them. Without this the plan is printed and nothing changes.
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        json: bool,
    },
    /// Withdraw a memory. The claim stops being returned, projected or read; the ledger keeps
    /// the evidence and records the withdrawal.
    Forget {
        #[arg(long)]
        project: String,
        /// The memory id, with or without a `memory:` prefix.
        #[arg(long)]
        id: String,
        /// Why. Required — a withdrawal with no reason is indistinguishable from corruption later.
        #[arg(long)]
        reason: String,
    },
    /// Write a project's memories and their evidence to a directory.
    Export {
        #[arg(long)]
        project: String,
        #[arg(long)]
        destination: PathBuf,
        #[arg(long, value_enum, default_value_t = brain_cli::ExportFormat::Both)]
        format: brain_cli::ExportFormat,
        /// Only memories valid from this RFC 3339 timestamp onwards.
        #[arg(long)]
        since: Option<String>,
    },
    Timeline {
        #[arg(long)]
        project: String,
        #[arg(long, value_enum, default_value_t = CliTimelineWindow::Week)]
        window: CliTimelineWindow,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        end: Option<String>,
        #[arg(long)]
        now: Option<String>,
        #[arg(long)]
        as_of: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
    Checkpoint {
        #[arg(long)]
        project: String,
        #[arg(long)]
        prompt: Option<String>,
        #[arg(long)]
        path: Vec<String>,
        #[arg(long)]
        as_of: Option<String>,
        #[arg(long)]
        max_tokens: Option<usize>,
    },
    Task {
        #[command(subcommand)]
        action: TaskCommand,
    },
    Preflight {
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: Option<uuid::Uuid>,
        #[arg(long, default_value = "HEAD")]
        source: String,
        #[arg(long)]
        target: String,
    },
    Diagnose {
        #[arg(long)]
        project: Option<String>,
    },
    Dashboard,
    /// Print a content hash of the build inputs.
    ///
    /// The deploy script records this so the dashboard can tell whether rebuilding would
    /// produce different binaries. Computing it here rather than reimplementing the walk in
    /// PowerShell keeps the two sides from ever disagreeing about what "unchanged" means.
    SourceFingerprint {
        #[arg(long)]
        source_root: String,
    },
    Rebuild {
        #[command(subcommand)]
        target: RebuildCommand,
    },
    Verify {
        #[command(subcommand)]
        target: VerifyCommand,
    },
    Backup {
        #[command(subcommand)]
        action: BackupCommand,
    },
    Restore {
        #[arg(long)]
        backup: PathBuf,
        #[arg(long)]
        destination: PathBuf,
    },
    Upgrade {
        #[command(subcommand)]
        action: UpgradeCommand,
    },
    Benchmark {
        #[command(subcommand)]
        action: Box<BenchmarkCommand>,
    },
    Providers {
        #[command(subcommand)]
        action: ProviderCommand,
    },
    Service {
        #[command(subcommand)]
        action: ServiceCommand,
    },
}

#[derive(Subcommand)]
enum BenchmarkCommand {
    /// Preserve the original generated-corpus scale benchmark.
    Scale {
        #[arg(long, value_enum, default_value_t = CliBenchmarkProfile::Smoke)]
        profile: CliBenchmarkProfile,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 42)]
        seed: u64,
    },
    /// Validate and freeze one zero-cost benchmark run.
    Preflight(Box<BenchmarkPreflightCommand>),
    /// Run the deterministic local retrieval gold-set evaluator.
    Retrieval {
        #[command(subcommand)]
        action: BenchmarkRetrievalCommand,
    },
    /// Preview by default. `--execute` is the only paid-session boundary.
    Run {
        #[arg(long)]
        project: String,
        #[arg(long)]
        run: uuid::Uuid,
        #[arg(long)]
        execute: bool,
    },
    Grade {
        #[command(subcommand)]
        action: BenchmarkGradeCommand,
    },
    Report {
        #[arg(long)]
        project: String,
        #[arg(long)]
        run: uuid::Uuid,
    },
    Show {
        #[arg(long)]
        project: String,
        #[arg(long)]
        run: Option<uuid::Uuid>,
    },
    Retire {
        #[arg(long)]
        project: String,
        #[arg(long)]
        run: uuid::Uuid,
        #[arg(long)]
        reason: String,
    },
    /// Show native production usage as an observational trend, never a savings claim.
    Production {
        #[arg(long)]
        project: String,
    },
}

#[derive(Subcommand)]
enum SessionsCommand {
    Status {
        #[arg(long)]
        project: String,
        #[arg(long)]
        active: bool,
        #[arg(long)]
        closed: bool,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum BenchmarkRetrievalCommand {
    /// Visible split used only for threshold calibration.
    Calibration(BenchmarkRetrievalArgs),
    /// Locked split used once after configuration is frozen.
    LockedTest(BenchmarkRetrievalArgs),
}

#[derive(Args)]
struct BenchmarkRetrievalArgs {
    #[arg(long)]
    project: String,
    #[arg(long)]
    gold: PathBuf,
    #[arg(long, required_unless_present = "run", conflicts_with = "run")]
    output: Option<PathBuf>,
    /// Attach the immutable retrieval cases and summary to an existing benchmark run.
    #[arg(long, conflicts_with = "output")]
    run: Option<uuid::Uuid>,
    /// Build one isolated deterministic ledger per gold case instead of reading production data.
    #[arg(long)]
    fixture: bool,
}

#[derive(Args)]
struct BenchmarkPreflightCommand {
    #[arg(long)]
    project: String,
    #[arg(long)]
    suite: PathBuf,
    #[arg(long)]
    repository: PathBuf,
    #[arg(long)]
    snapshot_source: Option<PathBuf>,
    #[arg(long)]
    run_id: Option<uuid::Uuid>,
    #[arg(long, default_value_t = 42)]
    seed: u64,
    #[arg(long, default_value_t = 5)]
    repeats: u32,
    /// Prepare the exact 20-session (two tasks x two harnesses x five conditions) schema smoke.
    #[arg(long, conflicts_with = "pilot")]
    smoke: bool,
    #[arg(long)]
    pilot: bool,
    /// Directory containing the ten audited C0-C4 condition profiles.
    #[arg(long)]
    condition_profiles: PathBuf,
    /// Immutable native launcher profiles used by both conditions.
    #[arg(long)]
    execution_templates: PathBuf,
}

#[derive(Subcommand)]
enum BenchmarkGradeCommand {
    Export {
        #[arg(long)]
        project: String,
        #[arg(long)]
        run: uuid::Uuid,
    },
    Import {
        #[arg(long)]
        project: String,
        #[arg(long)]
        run: uuid::Uuid,
        #[arg(long)]
        grades: PathBuf,
        #[arg(long)]
        grader: String,
        #[arg(long)]
        grader_version: String,
    },
}

#[derive(Subcommand)]
enum ServiceCommand {
    Install(Box<ServiceInstallCommand>),
    Start,
    Stop,
    Status,
    Uninstall,
}

#[derive(Args)]
struct ServiceInstallCommand {
    #[arg(long)]
    service_executable: Option<PathBuf>,
    #[arg(long)]
    brain_executable: Option<PathBuf>,
    #[arg(long)]
    backup_root: Option<PathBuf>,
    #[arg(long)]
    drill_root: Option<PathBuf>,
    #[arg(long)]
    install_hooks: bool,
    #[arg(long)]
    hook_executable: Option<PathBuf>,
    #[arg(long)]
    claude_settings: Option<PathBuf>,
    #[arg(long)]
    codex_settings: Option<PathBuf>,
}

#[derive(Subcommand)]
enum ProviderCommand {
    Status {
        #[arg(long)]
        project: String,
    },
    ConfigureLlmWiki {
        #[arg(long)]
        project: String,
        #[arg(long)]
        vault: PathBuf,
    },
    ConfigureCodegraph {
        #[arg(long)]
        project: String,
        #[arg(long)]
        executable: PathBuf,
        #[arg(long)]
        activation_report: PathBuf,
    },
    IndexCodegraph {
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: Option<uuid::Uuid>,
    },
    Disable {
        #[arg(long)]
        project: String,
        #[arg(value_enum)]
        provider: CliProviderKind,
    },
    Remove {
        #[arg(long)]
        project: String,
        #[arg(value_enum)]
        provider: CliProviderKind,
    },
}

#[derive(Subcommand)]
enum BackupCommand {
    Create {
        #[arg(long)]
        destination: PathBuf,
    },
    Verify {
        #[arg(long)]
        backup: PathBuf,
    },
    Maintain {
        #[arg(long)]
        root: PathBuf,
    },
    Prune {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        apply: bool,
    },
    Drill {
        #[arg(long)]
        backup: PathBuf,
        #[arg(long)]
        work_root: PathBuf,
    },
    DrillLatest {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        work_root: PathBuf,
    },
}

#[derive(Subcommand)]
enum UpgradeCommand {
    Check,
    Stage {
        #[arg(long)]
        destination: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliBenchmarkProfile {
    Smoke,
    Primary,
    Stress,
}

#[derive(Subcommand)]
enum RebuildCommand {
    Markdown {
        #[arg(long)]
        project: String,
    },
    BasicMemory {
        #[arg(long)]
        project: String,
    },
}

#[derive(Subcommand)]
enum VerifyCommand {
    Projections {
        #[arg(long)]
        project: String,
    },
    /// Walk a memory back to the events it cites.
    Memory {
        #[arg(long)]
        project: String,
        /// The memory id, with or without a `memory:` prefix.
        #[arg(long)]
        id: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum TaskCommand {
    Create {
        #[arg(long)]
        project: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        base: Option<String>,
        #[arg(long)]
        worktree_parent: Option<PathBuf>,
    },
    List {
        #[arg(long)]
        project: String,
        #[arg(long)]
        include_closed: bool,
    },
    Close {
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: uuid::Uuid,
    },
    Claim {
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: uuid::Uuid,
        #[arg(long, value_enum)]
        kind: CliClaimKind,
        #[arg(long)]
        value: String,
        #[arg(long)]
        symbol: Option<String>,
    },
    Claims {
        #[arg(long)]
        project: String,
    },
    ReleaseClaim {
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: uuid::Uuid,
        #[arg(long)]
        claim: uuid::Uuid,
    },
    Acquire {
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: uuid::Uuid,
        #[arg(long, value_enum)]
        harness: CliHarness,
        #[arg(long)]
        session: String,
    },
    Renew {
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: uuid::Uuid,
        #[arg(long, value_enum)]
        harness: CliHarness,
        #[arg(long)]
        session: String,
        #[arg(long)]
        generation: u64,
    },
    Release {
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: uuid::Uuid,
        #[arg(long, value_enum)]
        harness: CliHarness,
        #[arg(long)]
        session: String,
        #[arg(long)]
        generation: u64,
    },
    Handoff {
        #[arg(long)]
        project: String,
        #[arg(long)]
        handoff_id: uuid::Uuid,
        #[arg(long)]
        task: uuid::Uuid,
        #[arg(long, value_enum)]
        from_harness: CliHarness,
        #[arg(long)]
        from_session: String,
        #[arg(long)]
        generation: u64,
        #[arg(long, value_enum)]
        to_harness: CliHarness,
        #[arg(long)]
        to_session: String,
        #[arg(long)]
        checkpoint: String,
    },
    Leases {
        #[arg(long)]
        project: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum HookHarness {
    Claude,
    Codex,
}

#[derive(Clone, Copy, ValueEnum)]
enum McpHarness {
    Claude,
}

#[derive(Clone, Copy, ValueEnum)]
enum StatusHarness {
    Hermes,
}

#[derive(Clone, Copy, Default, ValueEnum)]
enum CliTimelineWindow {
    Day,
    #[default]
    Week,
    Month,
    Custom,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliClaimKind {
    File,
    Directory,
    Glob,
    Symbol,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliHarness {
    Claude,
    Codex,
    Hermes,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliProviderKind {
    Codegraph,
    LlmWiki,
}

fn main() -> Result<()> {
    std::thread::Builder::new()
        .name("brain-cli".to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(run)?
        .join()
        .map_err(|_| anyhow::anyhow!("brain CLI worker panicked"))?
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let brain_home = match cli.brain_home {
        Some(path) => path,
        None => BrainConfig::brain_home().context("resolve BRAIN_HOME")?,
    };
    match cli.command {
        Command::Register {
            path,
            claude_projects_root,
            no_discover_claude,
            codex_sessions_root,
            no_discover_codex,
            hermes_db,
            no_hermes,
        } => {
            let discovery_root = if no_discover_claude {
                None
            } else {
                claude_projects_root.or_else(default_claude_projects_root)
            };
            let codex_root = if no_discover_codex {
                None
            } else {
                codex_sessions_root.or_else(default_codex_sessions_root)
            };
            let hermes_database = if no_hermes {
                None
            } else {
                hermes_db.or_else(default_existing_hermes_database)
            };
            let result = register_project_with_sources(
                RegisterOptions {
                    brain_home,
                    project_path: path,
                    claude_projects_root: discovery_root,
                    explicit_claude_sources: Vec::new(),
                    pipe_name: None,
                },
                AgentSourceOptions {
                    configure_codex: true,
                    codex_sessions_root: codex_root,
                    explicit_codex_sources: Vec::new(),
                    configure_hermes: true,
                    hermes_database,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Status {
            project,
            harness: Some(StatusHarness::Hermes),
            hermes_db,
            json,
        } => {
            let project = project.as_deref().map(parse_project_id).transpose()?;
            let status = read_hermes_status(
                &brain_home,
                hermes_db.unwrap_or(default_hermes_database()?),
                project,
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!("harness: hermes");
                println!("activation: {}", status.activation);
                println!("project: {}", status.project_id.0);
                println!("database: {}", status.database_path.display());
                println!("observed_fingerprint: {}", status.observed_fingerprint);
                if let Some(reason) = status.reason {
                    println!("reason: {reason}");
                }
            }
        }
        Command::Sessions {
            action:
                SessionsCommand::Status {
                    project,
                    active,
                    closed,
                    limit,
                    cursor,
                    json,
                },
        } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project = config
                .projects
                .iter()
                .find(|project| project.project_id == project_id)
                .context("project is not registered")?;
            let ledger = EventLedger::open(&project.ledger_path, project_id)?;
            let page = read_session_status(
                &ledger,
                project_id,
                SessionStatusOptions {
                    filter: selected_session_filter(active, closed),
                    limit,
                    cursor,
                    now: time::OffsetDateTime::now_utc(),
                    stale_after: time::Duration::minutes(30),
                },
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&page)?);
            } else {
                for session in page.sessions {
                    println!(
                        "{} {} {:?} startup={:?} prompt={:?} mcp={:?} end={:?} capture={:?}",
                        session.harness.as_str(),
                        session.native_session_id,
                        session.state,
                        session.startup.state,
                        session.prompt_push.state,
                        session.mcp_pull.state,
                        session.session_end.state,
                        session.capture.state
                    );
                }
                if let Some(cursor) = page.next_cursor {
                    println!("next_cursor: {cursor}");
                }
            }
        }
        Command::Status {
            project,
            harness: None,
            hermes_db: _,
            json,
        } => {
            let project = project.as_deref().map(parse_project_id).transpose()?;
            let status = read_status(&brain_home, project)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!("project: {}", status.project_id.0);
                println!("root: {}", status.project_root.display());
                println!("events: {}", status.persisted_events);
                println!("sources: {}", status.source_count);
                println!("backlog_bytes: {}", status.backlog_bytes);
                println!("active_schema_drifts: {}", status.active_schema_drifts);
                println!("healthy: {}", status.healthy);
            }
        }
        Command::InstallHooks {
            harness: HookHarness::Claude,
            settings,
            hook_executable,
        } => {
            let result = install_claude_hooks(
                settings.unwrap_or(default_claude_settings()?),
                hook_executable.unwrap_or(default_hook_executable()?),
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::InstallHooks {
            harness: HookHarness::Codex,
            settings,
            hook_executable,
        } => {
            let result = install_codex_hooks(
                settings.unwrap_or(default_codex_hooks()?),
                hook_executable.unwrap_or(default_hook_executable()?),
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::InstallMcp {
            harness: McpHarness::Claude,
            claude_executable,
        } => {
            let executable = claude_executable.unwrap_or_else(|| PathBuf::from("claude"));
            let result = install_claude_mcp(&brain_home, &executable)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::UninstallHooks {
            harness: HookHarness::Claude,
            settings,
            hook_executable,
        } => {
            let result = uninstall_claude_hooks(
                settings.unwrap_or(default_claude_settings()?),
                hook_executable.unwrap_or(default_hook_executable()?),
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::UninstallMcp {
            harness: McpHarness::Claude,
            claude_executable,
        } => {
            let executable = claude_executable.unwrap_or_else(|| PathBuf::from("claude"));
            let result = uninstall_claude_mcp(&brain_home, &executable)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::UninstallHooks {
            harness: HookHarness::Codex,
            settings,
            hook_executable,
        } => {
            let result = uninstall_codex_hooks(
                settings.unwrap_or(default_codex_hooks()?),
                hook_executable.unwrap_or(default_hook_executable()?),
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Query {
            project,
            text,
            as_of,
            limit,
            rerank,
            expand,
        } => {
            let response = BrainQueryService::open(&brain_home)?.search(BrainSearchRequest {
                project,
                text,
                as_of,
                worktree_id: None,
                task_id: None,
                native_session_id: None,
                paths: Vec::new(),
                source: SourceSelector::All,
                limit,
                rerank,
                expand,
            })?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        Command::Timeline {
            project,
            window,
            start,
            end,
            now,
            as_of,
            limit,
        } => {
            let response =
                BrainQueryService::open(&brain_home)?.timeline(BrainTimelineRequest {
                    project,
                    window: window.into(),
                    start,
                    end,
                    now,
                    as_of,
                    source: SourceSelector::All,
                    limit,
                })?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        Command::Checkpoint {
            project,
            prompt,
            path,
            as_of,
            max_tokens,
        } => {
            let response =
                BrainQueryService::open(&brain_home)?.checkpoint(BrainCheckpointRequest {
                    project,
                    prompt,
                    paths: path,
                    as_of,
                    max_tokens,
                    harness: None,
                    native_session_id: None,
                })?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        Command::Task {
            action:
                TaskCommand::Create {
                    project,
                    title,
                    base,
                    worktree_parent,
                },
        } => {
            let result = TaskCommands::open(&brain_home, &project, worktree_parent)?
                .create(&title, base.as_deref())?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action:
                TaskCommand::Acquire {
                    project,
                    task,
                    harness,
                    session,
                },
        } => {
            let result =
                BrainQueryService::open(&brain_home)?.acquire_lease(BrainLeaseAcquireRequest {
                    project,
                    task_id: task,
                    owner: owner(harness, session),
                })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action:
                TaskCommand::Renew {
                    project,
                    task,
                    harness,
                    session,
                    generation,
                },
        } => {
            let result =
                BrainQueryService::open(&brain_home)?.renew_lease(BrainLeaseGenerationRequest {
                    project,
                    task_id: task,
                    owner: owner(harness, session),
                    generation,
                })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action:
                TaskCommand::Release {
                    project,
                    task,
                    harness,
                    session,
                    generation,
                },
        } => {
            let result = BrainQueryService::open(&brain_home)?.release_lease(
                BrainLeaseGenerationRequest {
                    project,
                    task_id: task,
                    owner: owner(harness, session),
                    generation,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action:
                TaskCommand::Handoff {
                    project,
                    handoff_id,
                    task,
                    from_harness,
                    from_session,
                    generation,
                    to_harness,
                    to_session,
                    checkpoint,
                },
        } => {
            let result =
                BrainQueryService::open(&brain_home)?.handoff_lease(BrainLeaseHandoffRequest {
                    project,
                    handoff_id,
                    task_id: task,
                    current_owner: owner(from_harness, from_session),
                    generation,
                    next_owner: owner(to_harness, to_session),
                    checkpoint,
                })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action: TaskCommand::Leases { project },
        } => {
            let result =
                BrainQueryService::open(&brain_home)?.leases(BrainLeasesRequest { project })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Preflight {
            project,
            task,
            source,
            target,
        } => {
            let result =
                BrainQueryService::open(&brain_home)?.preflight(BrainPreflightRequest {
                    project,
                    task_id: task,
                    source_ref: source,
                    target_ref: target,
                })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action:
                TaskCommand::List {
                    project,
                    include_closed,
                },
        } => {
            let result = TaskCommands::open(&brain_home, &project, None)?.list(include_closed)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action: TaskCommand::Close { project, task },
        } => {
            let result = TaskCommands::open(&brain_home, &project, None)?.close(task)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action:
                TaskCommand::Claim {
                    project,
                    task,
                    kind,
                    value,
                    symbol,
                },
        } => {
            let result = BrainQueryService::open(&brain_home)?.claim(BrainClaimRequest {
                project,
                task_id: task,
                claims: vec![PathClaimInput {
                    kind: kind.into(),
                    value,
                    symbol,
                }],
            })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action: TaskCommand::Claims { project },
        } => {
            let result =
                BrainQueryService::open(&brain_home)?.claims(BrainClaimsRequest { project })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action:
                TaskCommand::ReleaseClaim {
                    project,
                    task,
                    claim,
                },
        } => {
            let result =
                BrainQueryService::open(&brain_home)?.release_claim(BrainReleaseClaimRequest {
                    project,
                    task_id: task,
                    claim_id: claim,
                })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Diagnose { project } => {
            let project = project.as_deref().map(parse_project_id).transpose()?;
            let bundle = read_diagnostics(&brain_home, project)?;
            println!("{}", serde_json::to_string_pretty(&bundle)?);
        }
        Command::Dashboard => {
            let snapshot = read_dashboard(&brain_home)?;
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
        }
        Command::SourceFingerprint { source_root } => {
            let root = std::path::Path::new(&source_root);
            let fingerprint = source_fingerprint(root).ok_or_else(|| {
                anyhow::anyhow!("could not read build inputs under {}", root.display())
            })?;
            println!("{fingerprint}");
        }
        Command::Rebuild {
            target: RebuildCommand::Markdown { project },
        } => {
            let report = rebuild_markdown(&brain_home, parse_project_id(&project)?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Rebuild {
            target: RebuildCommand::BasicMemory { project },
        } => {
            let report = rebuild_basic_memory(&brain_home, parse_project_id(&project)?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Remember {
            project,
            title,
            content,
            evidence,
            session,
            supersedes,
            kind,
        } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let mut ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            let parse_ids = |values: &[String], label: &str| -> Result<Vec<uuid::Uuid>> {
                values
                    .iter()
                    .map(|value| {
                        uuid::Uuid::parse_str(
                            value
                                .trim()
                                .trim_start_matches("event:")
                                .trim_start_matches("memory:"),
                        )
                        .with_context(|| format!("{label} must be a UUID, got {value:?}"))
                    })
                    .collect()
            };
            // Derive the citations when none were given. This is the whole file-back fix: the
            // command was unusable while it demanded UUIDs by hand, so it was never used once.
            let evidence_ids = if evidence.is_empty() {
                let derived = brain_cli::derive_evidence(
                    &ledger,
                    project_id,
                    &format!("{title}. {content}"),
                    session.as_deref(),
                )?;
                anyhow::ensure!(
                    !derived.is_empty(),
                    "no turn in this project discusses that claim, so there is nothing to cite.                      Either the wording shares no vocabulary with the corpus, or the conclusion is                      genuinely new — in which case pass --evidence explicitly and say what it rests on"
                );
                println!("citing {} turn(s), derived from the claim:", derived.len());
                for id in &derived {
                    println!("  event:{id}");
                }
                derived
            } else {
                parse_ids(&evidence, "--evidence")?
            };
            let request = brain_cli::RememberRequest {
                project_id,
                worktree_id: project_config.worktree_id,
                kind: brain_domain::MemoryKind::from_name(&kind)
                    .with_context(|| format!("unknown memory kind {kind:?}"))?,
                title: &title,
                content: &content,
                evidence_ids,
                supersedes: parse_ids(&supersedes, "--supersedes")?,
                now: time::OffsetDateTime::now_utc(),
            };
            let filed = brain_cli::remember(&mut ledger, request)?;
            println!("{}", serde_json::to_string_pretty(&filed)?);
        }
        Command::Digest {
            project,
            write,
            json,
        } => {
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            // Omitting --project covers every registered project. That is what the scheduled task
            // uses, so registering a project brings it into the digest without reinstalling a task —
            // a schedule that silently skips new projects is worse than no schedule.
            let targets: Vec<ProjectId> = match &project {
                Some(path) => vec![ProjectRegistry::open(&brain_home)?.resolve(path)?],
                None => config
                    .projects
                    .iter()
                    .map(|project| project.project_id)
                    .collect(),
            };
            let mut digests = Vec::new();
            for project_id in targets {
                let project_config = config.project(Some(project_id))?;
                let ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
                let digest =
                    brain_cli::build_digest(&ledger, project_id, time::OffsetDateTime::now_utc())?;
                if !json {
                    print!("{}", brain_cli::render_digest(&digest));
                }
                if write {
                    // Appended to the same greppable log the projector writes, so a digest lands
                    // where someone already looks rather than in a file only the schedule knows.
                    let log =
                        brain_store::project_vault_root(&brain_home.join("vault"), project_id)
                            .join("log.md");
                    if let Some(parent) = log.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    use std::io::Write as _;
                    let mut file = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&log)?;
                    file.write_all(brain_cli::render_digest_markdown(&digest).as_bytes())?;
                    if !json {
                        println!("  appended to {}", log.display());
                    }
                }
                digests.push(digest);
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&digests)?);
            }
        }
        Command::Synthesize {
            project,
            generate,
            limit,
            json,
        } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            let report = if generate {
                let Some(brain_service::ConsolidationProviderConfig::Glm {
                    endpoint,
                    model,
                    api_key_env,
                    timeout_ms,
                    max_retries,
                }) = config.consolidation.clone()
                else {
                    anyhow::bail!(
                        "no consolidation provider is configured, so there is nothing to draft                          the prose with. `brain synthesize` without --generate still reports                          which subjects need it"
                    );
                };
                let provider = brain_context::GlmClient::new(brain_context::GlmConfig {
                    endpoint,
                    model,
                    api_key_env,
                    timeout: std::time::Duration::from_millis(timeout_ms),
                    max_retries,
                })?;
                // As in `revise`: a current-thread runtime for the one command that needs one,
                // rather than a reactor under every command that does not.
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?
                    .block_on(brain_cli::generate_synthesis(
                        &ledger,
                        &provider,
                        limit,
                        time::OffsetDateTime::now_utc(),
                    ))?
            } else {
                brain_cli::survey_synthesis(&ledger)?
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", brain_cli::render_synthesis(&report));
            }
        }
        Command::Jobs {
            project,
            retry_dead,
            json,
        } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let mut ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            let retried = if retry_dead {
                ledger.retry_dead_letter_jobs(time::OffsetDateTime::now_utc())?
            } else {
                0
            };
            let report = brain_cli::job_report(&ledger, retried)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", brain_cli::render_jobs(&report));
            }
        }
        Command::Replay {
            project,
            session,
            event,
            limit,
            offset,
            json,
        } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            if let Some(event_id) = event {
                let found = brain_cli::replay_event(&ledger, event_id)?;
                match (found, json) {
                    (Some(found), true) => println!("{}", serde_json::to_string_pretty(&found)?),
                    (Some(found), false) => println!(
                        "{}  {}
{}

{}",
                        found.occurred_at,
                        found.event_type,
                        found.source_locator,
                        found.content.unwrap_or_else(|| "(no text)".to_owned())
                    ),
                    (None, true) => println!("null"),
                    // Not an error. A citation can point at another project's ledger, and the
                    // honest answer there is "not here", never a cross-project read.
                    (None, false) => println!("event {event_id} is not in this project's ledger"),
                }
                return Ok(());
            }
            match session {
                None => {
                    let sessions = brain_cli::replay_sessions(&ledger, 25)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&sessions)?);
                    } else {
                        for session in sessions {
                            println!(
                                "  {:>6} events  {}  {}",
                                session.event_count,
                                session.last_event_at.date(),
                                session.native_session_id
                            );
                        }
                    }
                }
                Some(session) => {
                    let page = brain_cli::replay_page(
                        &ledger,
                        project_id,
                        &session,
                        limit.unwrap_or(brain_cli::REPLAY_DEFAULT_PAGE),
                        offset,
                    )?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&page)?);
                    } else {
                        print!("{}", brain_cli::render_replay_page(&page));
                    }
                }
            }
        }
        Command::Explain {
            project,
            text,
            limit,
            json,
        } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let mut ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            ledger.enable_vector_search(&brain_home);
            let report = brain_cli::explain_query(&ledger, project_id, &text, limit)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", brain_cli::render_explain(&report));
            }
        }
        Command::Reconcile {
            project,
            apply,
            json,
        } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let mut ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            if apply {
                let applied = brain_cli::apply_reconciliation(
                    &mut ledger,
                    project_id,
                    time::OffsetDateTime::now_utc(),
                )?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&applied)?);
                } else {
                    print!("{}", brain_cli::render_reconcile_apply(&applied));
                }
            } else {
                let report = brain_cli::propose_reconciliation(&ledger, project_id)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    print!("{}", brain_cli::render_reconcile(&report));
                }
            }
        }
        Command::Review {
            project,
            approve,
            reject,
            json,
        } => {
            anyhow::ensure!(
                !(approve.is_some() && reject.is_some()),
                "--approve and --reject are opposite rulings; pass one"
            );
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let mut ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            let now = time::OffsetDateTime::now_utc();
            let mut report = brain_cli::ReviewReport::default();
            if let Some(id) = approve {
                brain_cli::rule_on_memory(&mut ledger, id, true, now)?;
                report.approved = 1;
            }
            if let Some(id) = reject {
                brain_cli::rule_on_memory(&mut ledger, id, false, now)?;
                report.rejected = 1;
            }
            report.pending = brain_cli::pending_reviews(&ledger)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", brain_cli::render_review(&report));
                // Silence here would read as "nothing to review" when the truth is "nothing is
                // gated", and those are opposite states: the first means the queue is drained, the
                // second means no queue exists.
                if config.review.gated_kinds.is_empty() {
                    println!(
                        "\n  No kinds are gated, so nothing will ever appear here. Set\n  \
                         review.gated_kinds in {} — \"decision\" is the kind worth the friction.",
                        ServiceLaunchConfig::default_path(&brain_home).display()
                    );
                }
            }
        }
        Command::Revise {
            project,
            all,
            propose,
            apply,
            review_sheet,
            apply_reviewed,
            limit,
            json,
        } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let mut ledger = EventLedger::open(&project_config.ledger_path, project_id)?;

            // The reviewed path writes without proposing anything: the proposals already exist, a
            // human has ruled on them, and re-drafting would mean applying text nobody approved.
            if let Some(path) = apply_reviewed {
                anyhow::ensure!(
                    !apply && review_sheet.is_none(),
                    "--apply-reviewed is the approval; combining it with --apply or --review-sheet \
                     asks for two different write paths in one run"
                );
                let sheet: brain_cli::ReviewSheet =
                    serde_json::from_str(&std::fs::read_to_string(&path)?)
                        .with_context(|| format!("read review sheet {}", path.display()))?;
                anyhow::ensure!(
                    sheet.project_id == project_id.0,
                    "this sheet was written for project {}, not {}",
                    sheet.project_id,
                    project_id.0
                );
                let applied = brain_cli::apply_reviewed(
                    &mut ledger,
                    project_id,
                    &sheet,
                    time::OffsetDateTime::now_utc(),
                )?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&applied)?);
                } else {
                    print!("{}", brain_cli::render_merges(&applied));
                }
                return Ok(());
            }

            let report = brain_cli::propose_revisions(&ledger, project_id, all.then_some(0.0))?;
            if !propose {
                anyhow::ensure!(
                    !apply,
                    "--apply needs --propose: nothing can be written until a merge is drafted and checked"
                );
                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    print!("{}", brain_cli::render_revisions(&report));
                }
                return Ok(());
            }

            let Some(brain_service::ConsolidationProviderConfig::Glm {
                endpoint,
                model,
                api_key_env,
                timeout_ms,
                max_retries,
            }) = config.consolidation.clone()
            else {
                anyhow::bail!(
                    "no consolidation provider is configured, so there is nothing to draft the                      merge with. `brain revise` without --propose still lists the candidates"
                );
            };
            let provider = brain_context::GlmClient::new(brain_context::GlmConfig {
                endpoint,
                model,
                api_key_env,
                timeout: std::time::Duration::from_millis(timeout_ms),
                max_retries,
            })?;
            // `main` is synchronous — the CLI is otherwise entirely blocking, and making it async
            // to serve one command would put a reactor under every other one. A current-thread
            // runtime here is the smaller change.
            let merged = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(brain_cli::merge_candidates(
                    &mut ledger,
                    project_id,
                    &provider,
                    &report.candidates,
                    limit,
                    apply,
                    time::OffsetDateTime::now_utc(),
                ))?;
            if let Some(path) = review_sheet {
                let now = time::OffsetDateTime::now_utc();
                let sheet = brain_cli::review_sheet(project_id, &merged, now);
                let items = sheet.items.len();
                std::fs::write(&path, serde_json::to_string_pretty(&sheet)?)
                    .with_context(|| format!("write review sheet {}", path.display()))?;
                println!(
                    "  {items} proposal(s) written to {}\n\n  \
                     Set each item's \"decision\" to approve or reject, then:\n    \
                     brain revise --project {project} --apply-reviewed {}\n",
                    path.display(),
                    path.display()
                );
                return Ok(());
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&merged)?);
            } else {
                print!("{}", brain_cli::render_merges(&merged));
            }
        }
        Command::Evict {
            project,
            apply,
            json,
        } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let mut ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            let now = time::OffsetDateTime::now_utc();
            let plan = ledger.plan_eviction(now)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else {
                print!("{}", brain_cli::render_eviction(&plan));
            }
            if apply {
                // `apply_eviction` re-plans and enforces the gate itself, so this cannot retire
                // anything the printed plan did not cover, and cannot run early.
                let retired = ledger.apply_eviction(now, "brain evict")?;
                println!(
                    "
retired {retired} memories as tombstones"
                );
            }
        }
        Command::Lint {
            project,
            repair_dates,
            json,
        } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let mut ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            if repair_dates {
                let repaired =
                    brain_cli::repair_dates(&mut ledger, time::OffsetDateTime::now_utc())?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&repaired)?);
                } else {
                    print!("{}", brain_cli::render_date_repair(&repaired));
                }
                return Ok(());
            }
            let report =
                brain_cli::lint_project(&ledger, project_id, time::OffsetDateTime::now_utc())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", brain_cli::render_lint(&report));
            }
            // Findings that need a decision exit non-zero so this is usable in a check, while
            // observations — unrefreshed claims, islands, a running backfill — do not.
            if report.actionable {
                anyhow::bail!("lint found something that needs a decision");
            }
        }
        Command::Forget {
            project,
            id,
            reason,
        } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let memory_id = uuid::Uuid::parse_str(id.trim().trim_start_matches("memory:"))
                .context("memory id must be a UUID, optionally prefixed with `memory:`")?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let mut ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            let who = std::env::var("USERNAME")
                .or_else(|_| std::env::var("USER"))
                .unwrap_or_else(|_| "unknown".to_owned());
            let tombstone =
                ledger.forget_memory(memory_id, &reason, &who, time::OffsetDateTime::now_utc())?;
            println!("{}", serde_json::to_string_pretty(&tombstone)?);
            eprintln!(
                "Withdrawn. The memory will stop appearing in search, orientation and the vault 
                 at the next projection. Its evidence is untouched and the withdrawal is on record."
            );
        }
        Command::Export {
            project,
            destination,
            format,
            since,
        } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let since = since
                .as_deref()
                .map(|value| {
                    time::OffsetDateTime::parse(
                        value,
                        &time::format_description::well_known::Rfc3339,
                    )
                })
                .transpose()
                .context("--since must be an RFC 3339 timestamp")?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            let report =
                brain_cli::export_project(&ledger, project_id, &destination, format, since)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            // Evidence that will not resolve is the one thing an export must never hide, since
            // after it leaves here nothing can resolve it.
            if report.memories_missing_evidence > 0 {
                anyhow::bail!(
                    "{} memory(ies) cite evidence this ledger could not resolve",
                    report.memories_missing_evidence
                );
            }
        }
        Command::Verify {
            target: VerifyCommand::Memory { project, id, json },
        } => {
            // Accepts the same selector `brain query` does — a project path or an id — because
            // the id you have to hand when checking a claim is the memory's, not the project's.
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let memory_id = uuid::Uuid::parse_str(id.trim().trim_start_matches("memory:"))
                .context("memory id must be a UUID, optionally prefixed with `memory:`")?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            let report = brain_cli::verify_memory(&ledger, project_id, memory_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", brain_cli::render_provenance(&report));
            }
            // A citation that does not resolve is corruption of the one property this brain
            // sells, so it fails the command rather than being a line in the output.
            if !report.intact() {
                anyhow::bail!(
                    "{} citation(s) do not resolve to events in this ledger",
                    report.unresolved.len()
                );
            }
        }
        Command::Verify {
            target: VerifyCommand::Projections { project },
        } => {
            let report = verify_projections(&brain_home, parse_project_id(&project)?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.valid {
                anyhow::bail!("projection verification failed");
            }
        }
        Command::Backup {
            action: BackupCommand::Create { destination },
        } => {
            let report =
                BackupManager::create(&brain_home, destination, time::OffsetDateTime::now_utc())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Backup {
            action: BackupCommand::Verify { backup },
        } => {
            let report = BackupManager::verify(backup)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Backup {
            action: BackupCommand::Maintain { root },
        } => {
            let backup =
                BackupManager::create(&brain_home, &root, time::OffsetDateTime::now_utc())?;
            let retention =
                BackupManager::apply_retention(&root, RetentionPolicy::default(), false)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "backup": backup,
                    "retention": retention,
                }))?
            );
        }
        Command::Backup {
            action: BackupCommand::Prune { root, apply },
        } => {
            let report = BackupManager::apply_retention(root, RetentionPolicy::default(), !apply)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Backup {
            action: BackupCommand::Drill { backup, work_root },
        } => {
            let report =
                BackupManager::recovery_drill(backup, work_root, time::OffsetDateTime::now_utc())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.success {
                anyhow::bail!(
                    "recovery drill failed: {}",
                    report.error.as_deref().unwrap_or("unknown error")
                );
            }
        }
        Command::Backup {
            action: BackupCommand::DrillLatest { root, work_root },
        } => {
            let backup = BackupManager::latest_verified_backup(root)?;
            let report =
                BackupManager::recovery_drill(backup, work_root, time::OffsetDateTime::now_utc())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.success {
                anyhow::bail!(
                    "recovery drill failed: {}",
                    report.error.as_deref().unwrap_or("unknown error")
                );
            }
        }
        Command::Restore {
            backup,
            destination,
        } => {
            let report = BackupManager::restore_isolated(backup, destination)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Upgrade {
            action: UpgradeCommand::Check,
        } => {
            let report = UpgradeManager::check(&brain_home)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.compatible {
                anyhow::bail!("brain formats are not compatible with this binary");
            }
        }
        Command::Upgrade {
            action: UpgradeCommand::Stage { destination },
        } => {
            let report =
                UpgradeManager::stage(&brain_home, destination, time::OffsetDateTime::now_utc())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Benchmark { action } => match *action {
            BenchmarkCommand::Scale {
                profile,
                output,
                seed,
            } => {
                let report = benchmark_corpus(output, profile.into(), seed)?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if !report.passed {
                    anyhow::bail!("benchmark gates failed: {}", report.failures.join("; "));
                }
            }
            BenchmarkCommand::Preflight(arguments) => {
                let BenchmarkPreflightCommand {
                    project,
                    suite,
                    repository,
                    snapshot_source,
                    run_id,
                    seed,
                    repeats,
                    smoke,
                    pilot,
                    condition_profiles,
                    execution_templates,
                } = *arguments;
                let project_id = parse_project_id(&project)?;
                let report = preflight_benchmark(BenchmarkPreflightOptions {
                    brain_home: brain_home.clone(),
                    project_id,
                    suite_path: suite,
                    repository,
                    snapshot_source,
                    run_id,
                    seed,
                    repeats,
                    smoke,
                    pilot,
                    condition_profiles,
                    execution_templates,
                })?;
                BenchmarkArtifacts::new(&brain_home, project_id, report.run_id)?
                    .append_command("preflight", current_cli_argv())?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if !report.valid {
                    anyhow::bail!("benchmark preflight failed");
                }
            }
            BenchmarkCommand::Retrieval { action } => {
                let (arguments, split) = match action {
                    BenchmarkRetrievalCommand::Calibration(arguments) => {
                        (arguments, RetrievalSplit::Calibration)
                    }
                    BenchmarkRetrievalCommand::LockedTest(arguments) => {
                        (arguments, RetrievalSplit::LockedTest)
                    }
                };
                let project_id = ProjectRegistry::open(&brain_home)?.resolve(&arguments.project)?;
                let config_path = ServiceLaunchConfig::default_path(&brain_home);
                let config = ServiceLaunchConfig::load(&config_path)?;
                let executable = std::env::current_exe()?;
                let executable_sha256 = sha256_file(&executable)?;
                let config_sha256 = sha256_file(&config_path)?;
                let report = if let Some(run) = arguments.run {
                    let cases = read_gold_cases(&arguments.gold, split)?;
                    let report = if arguments.fixture {
                        evaluate_retrieval_fixture_cases(
                            project_id,
                            &cases,
                            &executable_sha256,
                            &config_sha256,
                        )?
                    } else {
                        let project = config
                            .projects
                            .iter()
                            .find(|project| project.project_id == project_id)
                            .context("project is not registered")?;
                        let ledger = EventLedger::open(&project.ledger_path, project_id)?;
                        evaluate_retrieval_cases(
                            &ledger,
                            project_id,
                            &cases,
                            &executable_sha256,
                            &config_sha256,
                        )?
                    };
                    let artifacts = BenchmarkArtifacts::new(&brain_home, project_id, run)?;
                    artifacts.append_command(
                        match split {
                            RetrievalSplit::Calibration => "retrieval_calibration",
                            RetrievalSplit::LockedTest => "retrieval_locked_test",
                        },
                        current_cli_argv(),
                    )?;
                    artifacts.attach_retrieval_report(&report)?;
                    report
                } else {
                    let options = RetrievalBenchmarkOptions {
                        gold_path: arguments.gold,
                        output_dir: arguments.output.context("output is required")?,
                        split,
                        executable_sha256,
                        config_sha256,
                    };
                    if arguments.fixture {
                        run_retrieval_fixture_benchmark(project_id, &options)?
                    } else {
                        let project = config
                            .projects
                            .iter()
                            .find(|project| project.project_id == project_id)
                            .context("project is not registered")?;
                        let ledger = EventLedger::open(&project.ledger_path, project_id)?;
                        run_retrieval_benchmark(&ledger, project_id, &options)?
                    }
                };
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
            BenchmarkCommand::Run {
                project,
                run,
                execute,
            } => {
                let project = parse_project_id(&project)?;
                BenchmarkArtifacts::new(&brain_home, project, run)?.append_command(
                    if execute {
                        "run_execute"
                    } else {
                        "run_preview"
                    },
                    current_cli_argv(),
                )?;
                let preview = if execute {
                    execute_benchmark_run(&brain_home, project, run)?
                } else {
                    preview_benchmark_run(&brain_home, project, run, false)?
                };
                println!("{}", serde_json::to_string_pretty(&preview)?);
            }
            BenchmarkCommand::Grade { action } => match action {
                BenchmarkGradeCommand::Export { project, run } => {
                    let artifacts =
                        BenchmarkArtifacts::new(&brain_home, parse_project_id(&project)?, run)?;
                    artifacts.append_command("grade_export", current_cli_argv())?;
                    let manifest = artifacts.manifest()?;
                    let task_text = artifacts.read_named_json("grading-tasks")?;
                    let exported = export_grading_bundle(&artifacts, &task_text, manifest.seed)?;
                    println!(
                        "{}",
                        serde_json::json!({
                            "sheet": exported.sheet,
                            "key": exported.key,
                            "rows": exported.rows
                        })
                    );
                }
                BenchmarkGradeCommand::Import {
                    project,
                    run,
                    grades,
                    grader,
                    grader_version,
                } => {
                    let artifacts =
                        BenchmarkArtifacts::new(&brain_home, parse_project_id(&project)?, run)?;
                    artifacts.append_command("grade_import", current_cli_argv())?;
                    let graded_at = time::OffsetDateTime::now_utc()
                        .format(&time::format_description::well_known::Rfc3339)?;
                    let imported =
                        import_grades(&artifacts, &grades, &grader, &grader_version, &graded_at)?;
                    println!("{}", serde_json::json!({ "imported": imported }));
                }
            },
            BenchmarkCommand::Report { project, run } => {
                let project_id = parse_project_id(&project)?;
                BenchmarkArtifacts::new(&brain_home, project_id, run)?
                    .append_command("report", current_cli_argv())?;
                let report = build_report_from_artifacts(&brain_home, project_id, run)?;
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
            BenchmarkCommand::Show { project, run } => {
                let project_id = parse_project_id(&project)?;
                let summary = if let Some(run) = run {
                    Some(BenchmarkArtifacts::new(&brain_home, project_id, run)?.summary()?)
                } else {
                    latest_summary(&brain_home, project_id)?
                };
                println!("{}", serde_json::to_string_pretty(&summary)?);
            }
            BenchmarkCommand::Retire {
                project,
                run,
                reason,
            } => {
                let artifacts =
                    BenchmarkArtifacts::new(&brain_home, parse_project_id(&project)?, run)?;
                artifacts.retire(&reason)?;
                println!("{}", serde_json::json!({ "run_id": run, "retired": true }));
            }
            BenchmarkCommand::Production { project } => {
                let project_id = parse_project_id(&project)?;
                let config =
                    ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
                let project = config
                    .projects
                    .iter()
                    .find(|project| project.project_id == project_id)
                    .context("project is not registered")?;
                let ledger = EventLedger::open(&project.ledger_path, project_id)?;
                let trend = brain_cli::production_token_trend(
                    &ledger,
                    project_id,
                    time::OffsetDateTime::now_utc(),
                )?;
                println!("{}", serde_json::to_string_pretty(&trend)?);
            }
        },
        Command::Providers {
            action: ProviderCommand::Status { project },
        } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&provider_status(&brain_home, &project)?)?
            );
        }
        Command::Providers {
            action: ProviderCommand::ConfigureLlmWiki { project, vault },
        } => {
            let report = configure_llm_wiki(&brain_home, &project, &vault)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Providers {
            action:
                ProviderCommand::ConfigureCodegraph {
                    project,
                    executable,
                    activation_report,
                },
        } => {
            let report =
                configure_codegraph(&brain_home, &project, &executable, &activation_report)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Providers {
            action: ProviderCommand::IndexCodegraph { project, task },
        } => {
            let report = index_codegraph(&brain_home, &project, task)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Providers {
            action: ProviderCommand::Disable { project, provider },
        } => {
            let report = disable_provider(&brain_home, &project, provider.into())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Providers {
            action: ProviderCommand::Remove { project, provider },
        } => {
            let report = remove_provider(&brain_home, &project, provider.into())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Service {
            action: ServiceCommand::Install(arguments),
        } => {
            let ServiceInstallCommand {
                service_executable,
                brain_executable,
                backup_root,
                drill_root,
                install_hooks,
                hook_executable,
                claude_settings,
                codex_settings,
            } = *arguments;
            let current_executable = std::env::current_exe()?;
            let binary_directory = current_executable
                .parent()
                .context("brain executable has no parent")?;
            let backup_root =
                backup_root.unwrap_or_else(|| sibling_path(&brain_home, "AgentBrainBackups"));
            let drill_root =
                drill_root.unwrap_or_else(|| sibling_path(&brain_home, "AgentBrainDrills"));
            let hook_executable = if install_hooks {
                Some(hook_executable.unwrap_or_else(|| binary_directory.join("brain-hook.exe")))
            } else {
                None
            };
            let claude_settings = if install_hooks {
                Some(match claude_settings {
                    Some(path) => path,
                    None => default_claude_settings()?,
                })
            } else {
                None
            };
            let codex_settings = if install_hooks {
                Some(match codex_settings {
                    Some(path) => path,
                    None => default_codex_hooks()?,
                })
            } else {
                None
            };
            let options = ServiceInstallOptions {
                brain_home: brain_home.clone(),
                service_executable: service_executable
                    .unwrap_or_else(|| binary_directory.join("brain-service.exe")),
                brain_executable: brain_executable.unwrap_or(current_executable),
                backup_root,
                drill_root,
                hook_executable,
                claude_settings,
                codex_settings,
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&install_windows_service(options)?)?
            );
        }
        Command::Service {
            action: ServiceCommand::Start,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&start_windows_service(&brain_home)?)?
        ),
        Command::Service {
            action: ServiceCommand::Stop,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&stop_windows_service(&brain_home)?)?
        ),
        Command::Service {
            action: ServiceCommand::Status,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&windows_service_status(&brain_home)?)?
        ),
        Command::Service {
            action: ServiceCommand::Uninstall,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&uninstall_windows_service(&brain_home)?)?
        ),
    }
    Ok(())
}

impl From<CliProviderKind> for brain_cli::ProviderKind {
    fn from(value: CliProviderKind) -> Self {
        match value {
            CliProviderKind::Codegraph => Self::Codegraph,
            CliProviderKind::LlmWiki => Self::LlmWiki,
        }
    }
}

impl From<CliTimelineWindow> for TimelineWindow {
    fn from(value: CliTimelineWindow) -> Self {
        match value {
            CliTimelineWindow::Day => Self::Day,
            CliTimelineWindow::Week => Self::Week,
            CliTimelineWindow::Month => Self::Month,
            CliTimelineWindow::Custom => Self::Custom,
        }
    }
}

impl From<CliClaimKind> for ClaimKind {
    fn from(value: CliClaimKind) -> Self {
        match value {
            CliClaimKind::File => Self::File,
            CliClaimKind::Directory => Self::Directory,
            CliClaimKind::Glob => Self::Glob,
            CliClaimKind::Symbol => Self::Symbol,
        }
    }
}

impl From<CliBenchmarkProfile> for BenchmarkProfile {
    fn from(value: CliBenchmarkProfile) -> Self {
        match value {
            CliBenchmarkProfile::Smoke => Self::Smoke,
            CliBenchmarkProfile::Primary => Self::Primary,
            CliBenchmarkProfile::Stress => Self::Stress,
        }
    }
}

fn owner(harness: CliHarness, native_session_id: String) -> SessionIdentity {
    SessionIdentity {
        harness: match harness {
            CliHarness::Claude => Harness::ClaudeCode,
            CliHarness::Codex => Harness::Codex,
            CliHarness::Hermes => Harness::Hermes,
        },
        native_session_id,
    }
}

fn selected_session_filter(active: bool, closed: bool) -> SessionFilter {
    if active && closed {
        SessionFilter::All
    } else if active {
        SessionFilter::Active
    } else if closed {
        SessionFilter::Closed
    } else {
        SessionFilter::All
    }
}

fn parse_project_id(value: &str) -> Result<ProjectId> {
    Ok(ProjectId(
        uuid::Uuid::parse_str(value).with_context(|| format!("invalid project ID {value}"))?,
    ))
}

fn current_cli_argv() -> Vec<String> {
    std::env::args_os()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}

fn default_claude_projects_root() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|home| home.join(".claude").join("projects"))
        .filter(|path| path.is_dir())
}

fn sibling_path(brain_home: &std::path::Path, name: &str) -> PathBuf {
    brain_home
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(name)
}

fn default_codex_sessions_root() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|home| home.join(".codex").join("sessions"))
        .filter(|path| path.is_dir())
}

fn default_existing_hermes_database() -> Option<PathBuf> {
    default_hermes_database().ok().filter(|path| path.is_file())
}

fn default_claude_settings() -> Result<PathBuf> {
    let home = std::env::var_os("USERPROFILE").context("USERPROFILE is unavailable")?;
    Ok(PathBuf::from(home).join(".claude").join("settings.json"))
}

fn default_codex_hooks() -> Result<PathBuf> {
    let home = std::env::var_os("USERPROFILE").context("USERPROFILE is unavailable")?;
    Ok(PathBuf::from(home).join(".codex").join("hooks.json"))
}

fn default_hermes_database() -> Result<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is unavailable")?;
    Ok(PathBuf::from(local).join("hermes").join("state.db"))
}

fn default_hook_executable() -> Result<PathBuf> {
    let current = std::env::current_exe().context("resolve brain executable")?;
    let name = if cfg!(windows) {
        "brain-hook.exe"
    } else {
        "brain-hook"
    };
    Ok(current.with_file_name(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_status_accepts_active_and_closed_together() {
        let cli = Cli::try_parse_from([
            "brain",
            "sessions",
            "status",
            "--project",
            "019fcd85-f41b-77b2-a4c0-618c28fe1d6b",
            "--active",
            "--closed",
            "--json",
        ])
        .expect("active and closed are cumulative status selections");

        assert!(matches!(
            cli.command,
            Command::Sessions {
                action: SessionsCommand::Status {
                    active: true,
                    closed: true,
                    json: true,
                    ..
                }
            }
        ));
    }

    #[test]
    fn active_and_closed_select_the_combined_session_view() {
        assert_eq!(selected_session_filter(true, true), SessionFilter::All);
    }
}
