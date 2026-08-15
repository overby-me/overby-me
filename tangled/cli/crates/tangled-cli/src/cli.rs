use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug, Clone)]
#[command(name = "tangled", author, version, about = "Tangled CLI", long_about = None)]
pub struct Cli {
    /// Config file path override
    #[arg(long, global = true)]
    pub config: Option<String>,

    /// Use named profile
    #[arg(long, global = true)]
    pub profile: Option<String>,

    /// Output format
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,

    /// Verbose output
    #[arg(long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Quiet output
    #[arg(long, global = true, default_value_t = false)]
    pub quiet: bool,

    /// Disable colors
    #[arg(long, global = true, default_value_t = false)]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum OutputFormat {
    Json,
    Table,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Authentication commands
    #[command(subcommand)]
    Auth(AuthCommand),
    /// Repository commands
    #[command(subcommand)]
    Repo(RepoCommand),
    /// Issue commands
    #[command(subcommand)]
    Issue(IssueCommand),
    /// Pull request commands
    #[command(subcommand)]
    Pr(PrCommand),
    /// Knot management commands
    #[command(subcommand)]
    Knot(KnotCommand),
    /// Spindle integration commands
    #[command(subcommand)]
    Spindle(SpindleCommand),
    /// Open in browser
    Browse(BrowseArgs),
    /// Make raw authenticated XRPC API calls
    #[command(subcommand)]
    Api(ApiCommand),
    /// Show cross-repo dashboard of issues and PRs needing attention
    Status,
    /// git credential helper: mints a short-lived knot push token
    #[command(name = "git-credential")]
    GitCredential(GitCredentialArgs),
}

#[derive(Args, Debug, Clone)]
pub struct GitCredentialArgs {
    /// git passes one of: get, store, erase
    pub operation: String,
    /// Mint the token for this knot instead of the host git asked about.
    /// Needed when pushing through something that is not the knot itself,
    /// such as a local josh-proxy on localhost.
    #[arg(long)]
    pub knot: Option<String>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum AuthCommand {
    /// Login with Bluesky credentials
    Login(AuthLoginArgs),
    /// Login via browser (OAuth)
    LoginBrowser(AuthLoginBrowserArgs),
    /// Show authentication status
    Status,
    /// Logout and clear session
    Logout,
}

#[derive(Args, Debug, Clone)]
pub struct AuthLoginArgs {
    /// Bluesky handle (e.g. user.bsky.social)
    #[arg(long)]
    pub handle: Option<String>,
    /// Password (will prompt if omitted)
    #[arg(long)]
    pub password: Option<String>,
    /// PDS URL (default: https://bsky.social)
    #[arg(long)]
    pub pds: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct AuthLoginBrowserArgs {
    /// Bluesky handle or PDS URL
    pub handle: Option<String>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum RepoCommand {
    /// List repositories
    List(RepoListArgs),
    /// Create repository
    Create(RepoCreateArgs),
    /// Clone repository
    Clone(RepoCloneArgs),
    /// Show repository information
    Info(RepoInfoArgs),
    /// Edit repository settings
    Edit(RepoEditArgs),
    /// Delete a repository
    Delete(RepoDeleteArgs),
    /// Star a repository
    Star(RepoRefArgs),
    /// Unstar a repository
    Unstar(RepoRefArgs),
    /// Fork a repository
    Fork(RepoForkArgs),
}

#[derive(Args, Debug, Clone)]
pub struct RepoListArgs {
    #[arg(long)]
    pub knot: Option<String>,
    #[arg(long)]
    pub user: Option<String>,
    #[arg(long, default_value_t = false)]
    pub starred: bool,
    /// Tangled API base URL (overrides env)
    #[arg(long)]
    pub base: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct RepoCreateArgs {
    pub name: String,
    #[arg(long)]
    pub knot: Option<String>,
    #[arg(long, default_value_t = false)]
    pub private: bool,
    #[arg(long)]
    pub description: Option<String>,
    #[arg(long, default_value_t = false)]
    pub init: bool,
}

#[derive(Args, Debug, Clone)]
pub struct RepoCloneArgs {
    pub repo: String,
    #[arg(long, default_value_t = false)]
    pub https: bool,
    #[arg(long)]
    pub depth: Option<usize>,
}

#[derive(Args, Debug, Clone)]
pub struct RepoInfoArgs {
    pub repo: String,
    #[arg(long, default_value_t = false)]
    pub stats: bool,
    #[arg(long, default_value_t = false)]
    pub contributors: bool,
}

#[derive(Args, Debug, Clone)]
pub struct RepoDeleteArgs {
    pub repo: String,
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

#[derive(Args, Debug, Clone)]
pub struct RepoEditArgs {
    pub repo: String,
    /// New description
    #[arg(long)]
    pub description: Option<String>,
    /// Make repository private
    #[arg(long, conflicts_with = "public")]
    pub private: bool,
    /// Make repository public
    #[arg(long, conflicts_with = "private")]
    pub public: bool,
}

#[derive(Args, Debug, Clone)]
pub struct RepoRefArgs {
    pub repo: String,
}

#[derive(Args, Debug, Clone)]
pub struct RepoForkArgs {
    /// Source repo: <owner>/<name>
    pub repo: String,
    /// Name for the forked repo (defaults to source name)
    #[arg(long)]
    pub name: Option<String>,
    /// Target knot hostname
    #[arg(long)]
    pub knot: Option<String>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum IssueCommand {
    List(IssueListArgs),
    Create(IssueCreateArgs),
    Show(IssueShowArgs),
    Edit(IssueEditArgs),
    Comment(IssueCommentArgs),
    /// Close an issue
    Close(IssueCloseArgs),
    /// Reopen an issue
    Reopen(IssueReopenArgs),
    /// Delete an issue
    Delete(IssueDeleteArgs),
}

#[derive(Args, Debug, Clone)]
pub struct IssueListArgs {
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub state: Option<String>,
    #[arg(long)]
    pub author: Option<String>,
    #[arg(long)]
    pub label: Option<String>,
    #[arg(long)]
    pub assigned: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct IssueCreateArgs {
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub body: Option<String>,
    #[arg(long)]
    pub label: Option<Vec<String>>,
    #[arg(long, value_name = "HANDLE")]
    pub assign: Option<Vec<String>>,
}

#[derive(Args, Debug, Clone)]
pub struct IssueShowArgs {
    pub id: String,
    #[arg(long, default_value_t = false)]
    pub comments: bool,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct IssueEditArgs {
    pub id: String,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub body: Option<String>,
    #[arg(long)]
    pub state: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct IssueCommentArgs {
    pub id: String,
    #[arg(long)]
    pub body: Option<String>,
    #[arg(long, default_value_t = false)]
    pub close: bool,
}

#[derive(Args, Debug, Clone)]
pub struct IssueCloseArgs {
    pub id: String,
    /// Add a comment when closing
    #[arg(long)]
    pub comment: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct IssueReopenArgs {
    pub id: String,
    /// Add a comment when reopening
    #[arg(long)]
    pub comment: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct IssueDeleteArgs {
    pub id: String,
    /// Skip confirmation prompt
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum PrCommand {
    List(PrListArgs),
    Create(PrCreateArgs),
    Show(PrShowArgs),
    Review(PrReviewArgs),
    Merge(PrMergeArgs),
    /// Add a comment to a pull request
    Comment(PrCommentArgs),
    /// View pull request diff
    Diff(PrDiffArgs),
    /// Close a pull request without merging
    Close(PrCloseArgs),
    /// Reopen a closed pull request
    Reopen(PrReopenArgs),
    /// Apply a PR's patch locally for testing
    Checkout(PrCheckoutArgs),
}

#[derive(Args, Debug, Clone)]
pub struct PrListArgs {
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub state: Option<String>,
    #[arg(long)]
    pub author: Option<String>,
    #[arg(long)]
    pub reviewer: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct PrCreateArgs {
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub base: Option<String>,
    #[arg(long)]
    pub head: Option<String>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub body: Option<String>,
    #[arg(long, default_value_t = false)]
    pub draft: bool,
}

#[derive(Args, Debug, Clone)]
pub struct PrShowArgs {
    pub id: String,
    #[arg(long, default_value_t = false)]
    pub diff: bool,
    #[arg(long, default_value_t = false)]
    pub comments: bool,
    #[arg(long, default_value_t = false)]
    pub checks: bool,
}

#[derive(Args, Debug, Clone)]
pub struct PrReviewArgs {
    pub id: String,
    #[arg(long, default_value_t = false)]
    pub approve: bool,
    #[arg(long, default_value_t = false)]
    pub request_changes: bool,
    #[arg(long)]
    pub comment: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct PrMergeArgs {
    pub id: String,
}

#[derive(Args, Debug, Clone)]
pub struct PrCommentArgs {
    pub id: String,
    #[arg(long)]
    pub body: String,
}

#[derive(Args, Debug, Clone)]
pub struct PrDiffArgs {
    pub id: String,
}

#[derive(Args, Debug, Clone)]
pub struct PrCloseArgs {
    pub id: String,
    /// Add a comment when closing
    #[arg(long)]
    pub comment: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct PrReopenArgs {
    pub id: String,
    /// Add a comment when reopening
    #[arg(long)]
    pub comment: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct PrCheckoutArgs {
    pub id: String,
    /// Branch name to create (default: pr/<rkey>)
    #[arg(long)]
    pub branch: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct BrowseArgs {
    /// Repo, issue, or PR reference (e.g. owner/repo, or rkey)
    pub target: Option<String>,
    /// Open issues tab
    #[arg(long, default_value_t = false)]
    pub issues: bool,
    /// Open pull requests tab
    #[arg(long, default_value_t = false)]
    pub prs: bool,
    /// Print URL instead of opening browser
    #[arg(long, short = 'n', default_value_t = false)]
    pub no_browser: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ApiCommand {
    /// Make an authenticated XRPC GET request
    Get(ApiGetArgs),
    /// Make an authenticated XRPC POST request
    Post(ApiPostArgs),
}

#[derive(Args, Debug, Clone)]
pub struct ApiGetArgs {
    /// XRPC method name (e.g. com.atproto.repo.listRecords)
    pub method: String,
    /// Query parameters as key=value pairs
    #[arg(long = "param", value_name = "KEY=VALUE")]
    pub params: Vec<String>,
    /// Target PDS instead of Tangled API
    #[arg(long)]
    pub pds: bool,
}

#[derive(Args, Debug, Clone)]
pub struct ApiPostArgs {
    /// XRPC method name (e.g. sh.tangled.repo.create)
    pub method: String,
    /// JSON body from file (use - for stdin)
    #[arg(long, value_name = "FILE")]
    pub input: Option<String>,
    /// Query parameters as key=value pairs
    #[arg(long = "param", value_name = "KEY=VALUE")]
    pub params: Vec<String>,
    /// Target PDS instead of Tangled API
    #[arg(long)]
    pub pds: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum KnotCommand {
    /// Migrate a repository to another knot
    Migrate(KnotMigrateArgs),
}

#[derive(Args, Debug, Clone)]
pub struct KnotMigrateArgs {
    /// Repo to migrate: <owner>/<name> (owner defaults to your handle)
    #[arg(long)]
    pub repo: String,
    /// Target knot hostname (e.g. knot1.tangled.sh)
    #[arg(long, value_name = "HOST")]
    pub to: String,
    /// Use HTTPS source when seeding new repo
    #[arg(long, default_value_t = true)]
    pub https: bool,
    /// Update PDS record knot field after seeding
    #[arg(long, default_value_t = true)]
    pub update_record: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SpindleCommand {
    /// List pipeline configurations
    List(SpindleListArgs),
    /// List pipeline runs (workflow executions)
    Runs(SpindleRunsArgs),
    Config(SpindleConfigArgs),
    Run(SpindleRunArgs),
    Logs(SpindleLogsArgs),
    /// Secrets management
    #[command(subcommand)]
    Secret(SpindleSecretCommand),
}

#[derive(Args, Debug, Clone)]
pub struct SpindleListArgs {
    #[arg(long)]
    pub repo: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct SpindleRunsArgs {
    /// Repo: <owner>/<name>
    #[arg(long)]
    pub repo: Option<String>,
    /// Filter by status (pending, running, success, failed, timeout, cancelled)
    #[arg(long)]
    pub status: Option<String>,
    /// Maximum number of runs to show
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
}

#[derive(Args, Debug, Clone)]
pub struct SpindleConfigArgs {
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub url: Option<String>,
    #[arg(long, default_value_t = false)]
    pub enable: bool,
    #[arg(long, default_value_t = false)]
    pub disable: bool,
}

#[derive(Args, Debug, Clone)]
pub struct SpindleRunArgs {
    #[arg(long)]
    pub repo: Option<String>,
    /// Branch to run for (default: main)
    #[arg(long)]
    pub branch: Option<String>,
    /// Commit to run. Defaults to the branch tip on the knot.
    #[arg(long)]
    pub sha: Option<String>,
    /// Stream the logs once it starts
    #[arg(long, default_value_t = false)]
    pub wait: bool,
}

#[derive(Args, Debug, Clone)]
pub struct SpindleLogsArgs {
    /// Pipeline id, as shown by `spindle runs`
    pub pipeline: String,
    /// Only this workflow (e.g. publish.yml); repeatable
    #[arg(long)]
    pub workflow: Vec<String>,
    /// Repo: <owner>/<name>
    #[arg(long)]
    pub repo: Option<String>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SpindleSecretCommand {
    /// List secrets for a repo
    List(SpindleSecretListArgs),
    /// Add or update a secret
    Add(SpindleSecretAddArgs),
    /// Remove a secret
    Remove(SpindleSecretRemoveArgs),
}

#[derive(Args, Debug, Clone)]
pub struct SpindleSecretListArgs {
    /// Repo: <owner>/<name>
    #[arg(long)]
    pub repo: String,
}

#[derive(Args, Debug, Clone)]
pub struct SpindleSecretAddArgs {
    /// Repo: <owner>/<name>
    #[arg(long)]
    pub repo: String,
    /// Secret key
    #[arg(long)]
    pub key: String,
    /// Secret value (use '@filename' to read from file, '-' to read from stdin)
    #[arg(long)]
    pub value: String,
}

#[derive(Args, Debug, Clone)]
pub struct SpindleSecretRemoveArgs {
    /// Repo: <owner>/<name>
    #[arg(long)]
    pub repo: String,
    /// Secret key
    #[arg(long)]
    pub key: String,
}
