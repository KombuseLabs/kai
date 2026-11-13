//! `kai ticket`: the tickets in the store, and whether kai may use them.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::{Args, Subcommand};
use types::{ActorAccess, IssueState, Outcome, RepoName, Ticket, Verdict};

use super::{Context, Failure};
use crate::settings;

#[derive(Subcommand)]
pub(crate) enum TicketCommand {
    /// Lists the tickets in the store: issue, state, labels, title
    List,
    /// Says for each ticket in the store whether kai may use it, and why not, judged from what GitHub said
    Qualify,
    /// Clones a ticket's repository at its ref and fetches its dependencies in a microVM, then builds them in a second one without network; prints the two layers
    Deps(DepsArgs),
}

#[derive(Args)]
pub(crate) struct DepsArgs {
    /// The ticket's issue number on the board
    #[arg(
        value_name = "NUMBER",
        required_unless_present = "repo",
        conflicts_with = "repo"
    )]
    number: Option<u64>,

    /// Instead of a ticket: a public repository on GitHub, as owner/name
    #[arg(long, value_name = "OWNER/NAME", requires = "sha")]
    repo: Option<RepoName>,

    /// With --repo: the commit, a full sha
    #[arg(long = "ref", value_name = "SHA", requires = "repo")]
    sha: Option<String>,

    /// Where kai keeps its VMs' files; without it, vm.cache in kai.toml, else ~/.cache/kai/vm
    #[arg(long, env = "KAI_VM_CACHE", value_name = "DIR")]
    cache: Option<PathBuf>,
}

pub(crate) fn run(command: &TicketCommand, context: &Context<'_>) -> Result<(), Failure> {
    if let TicketCommand::Deps(args) = command {
        return deps(args, context);
    }
    let (tickets, access) = tickets(&context.store)?;
    match command {
        TicketCommand::List => print_list(&tickets),
        TicketCommand::Qualify => print_qualify(&tickets, &access),
        TicketCommand::Deps(_) => {}
    }
    Ok(())
}

/// The repository and commit to fetch: the ticket's, once its static rules
/// refuse it for nothing, or those given.
fn target(args: &DepsArgs, context: &Context<'_>) -> Result<(RepoName, String), Failure> {
    if let (Some(repo), Some(sha)) = (&args.repo, &args.sha) {
        let full = sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit());
        if !full {
            return Err(Failure::Usage(anyhow::anyhow!(
                "--ref takes a full sha, not {sha}"
            )));
        }
        return Ok((repo.clone(), sha.to_ascii_lowercase()));
    }
    let number = args.number.unwrap_or_default();
    let (tickets, access) = tickets(&context.store)?;
    let found: Vec<&Ticket> = tickets
        .iter()
        .filter(|t| t.issue.number == number)
        .collect();
    let ticket = match found.as_slice() {
        [ticket] => *ticket,
        [] => {
            return Err(Failure::Usage(anyhow::anyhow!(
                "the store holds no ticket #{number}; `kai github sync` fetches them"
            )));
        }
        _ => {
            return Err(Failure::Usage(anyhow::anyhow!(
                "the store holds #{number} of several boards; use --repo and --ref"
            )));
        }
    };
    let qualification = ::engine::qualify::qualify(ticket, board_access(&access, ticket));
    if qualification.verdict() == Verdict::Refused {
        let why: Vec<String> = qualification
            .findings
            .iter()
            .filter_map(|finding| match &finding.outcome {
                Outcome::Fail(why) => Some(format!("{}: {why}", finding.rule)),
                _ => None,
            })
            .collect();
        return Err(Failure::Usage(anyhow::anyhow!(
            "{} is refused: {}",
            ticket.issue,
            why.join("; ")
        )));
    }
    let form = ticket.form.as_ref();
    let repo = form
        .and_then(|form| form.repo.as_deref())
        .and_then(|repo| repo.parse::<RepoName>().ok());
    let sha = form.and_then(|form| form.base_ref.clone());
    match (repo, sha) {
        (Some(repo), Some(sha)) => Ok((repo, sha.to_ascii_lowercase())),
        _ => Err(Failure::Usage(anyhow::anyhow!(
            "{} names no repository and ref kai can read",
            ticket.issue
        ))),
    }
}

/// `kai ticket deps`: the two layers, made or found in the cache.
fn deps(args: &DepsArgs, context: &Context<'_>) -> Result<(), Failure> {
    let (repo, sha) = target(args, context)?;
    let cache =
        settings::vm_cache(args.cache.as_deref(), context.config).map_err(Failure::Usage)?;
    // Absolute, so that the line printed last works from any directory.
    let cache = std::path::absolute(&cache)
        .with_context(|| format!("cannot make {} absolute", cache.display()))?;
    let ecosystem = &::engine::deps::RUST;
    let recipe = ::vm::image::recipe(ecosystem.recipe).context("kai's Rust recipe")?;
    let (image, info) = match ::vm::image::built(recipe, &cache).context("cannot read the cache")? {
        Some(found) => found,
        None => {
            return Err(Failure::Run(anyhow::anyhow!(
                "the image of the recipe `{}` is not built; `kai vm image build {}` builds it",
                recipe.name,
                recipe.name
            )));
        }
    };
    let layers = context.runtime.block_on(async {
        let launch = ::vm::Launch::from_cache(&cache).await?;
        let request = ::engine::deps::Request {
            ecosystem,
            repo: &repo,
            sha: &sha,
            image: (&image, &info),
            cache: &cache,
            fence: None,
        };
        ::engine::deps::deps(&launch, &request)
            .await
            .map_err(anyhow::Error::from)
    });
    let layers = layers.with_context(|| format!("no layers for {repo} at {sha}"))?;
    for (what, layer) in [("fetch", &layers.fetch), ("build", &layers.build)] {
        let how = if layer.cached { "cached" } else { "made" };
        println!("{what}: {how}, {} {}", layer.key, layer.file.display());
    }
    let mut line = run_line(recipe.name, &cache, &layers.fetch.file, &layers.build.file);
    line.push(b'\n');
    std::io::stdout()
        .write_all(&line)
        .context("cannot write to standard output")?;
    Ok(())
}

/// The `kai vm run` line that works on the two layers, with the cache they
/// lie in, every word quoted for a POSIX shell so that the line can be
/// copied as it is; `<command>` is the reader's to fill in. Bytes, since a
/// path need not be UTF-8.
fn run_line(recipe: &str, cache: &Path, fetch: &Path, build: &Path) -> Vec<u8> {
    let mut line = b"kai vm run --cache ".to_vec();
    line.extend(shell_word(cache.as_os_str().as_bytes()));
    line.extend_from_slice(b" --image ");
    line.extend(shell_word(recipe.as_bytes()));
    for layer in [fetch, build] {
        line.extend_from_slice(b" --layer ");
        line.extend(shell_word(layer.as_os_str().as_bytes()));
    }
    line.extend_from_slice(b" --cwd /src -- <command>");
    line
}

/// `word` as one word for a shell, its bytes unchanged: as it is when it
/// holds nothing a shell reads specially, else between single quotes,
/// with each `'` and `\` between double quotes instead. POSIX shells (sh,
/// bash, dash, zsh) read that as `word`, and so does fish, which reads
/// `\'` and `\\` even between single quotes.
fn shell_word(word: &[u8]) -> Vec<u8> {
    let plain = !word.is_empty()
        && word
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"%+,-./:=@_".contains(byte));
    if plain {
        return word.to_vec();
    }
    let mut quoted = Vec::new();
    let mut open = false;
    for &byte in word {
        if byte == b'\'' || byte == b'\\' {
            if open {
                quoted.push(b'\'');
                open = false;
            }
            quoted.extend_from_slice(if byte == b'\'' { b"\"'\"" } else { b"\"\\\\\"" });
        } else {
            if !open {
                quoted.push(b'\'');
                open = true;
            }
            quoted.push(byte);
        }
    }
    if open {
        quoted.push(b'\'');
    }
    if quoted.is_empty() {
        quoted.extend_from_slice(b"''");
    }
    quoted
}

/// What GitHub said of the access of each board's label actors.
type BoardAccess = BTreeMap<RepoName, Vec<ActorAccess>>;

/// Every issue in the store, read as a ticket, and the access of each
/// board's label actors; a hint on standard error when there is no ticket.
fn tickets(store_path: &Path) -> anyhow::Result<(Vec<Ticket>, BoardAccess)> {
    let store = super::store::open(store_path)?;
    let tickets: Vec<Ticket> = store
        .issues()
        .context("cannot read the store")?
        .iter()
        .map(::engine::ticket::from_issue)
        .collect();
    if tickets.is_empty() {
        eprintln!(
            "kai: the store {} holds no tickets; `kai github sync` fetches them",
            store_path.display()
        );
    }
    let mut access = BoardAccess::new();
    for ticket in &tickets {
        let board = &ticket.issue.repo;
        if !access.contains_key(board) {
            let answers = store.access(board).context("cannot read the store")?;
            access.insert(board.clone(), answers);
        }
    }
    Ok((tickets, access))
}

/// The access of the label actors of `ticket`'s board.
fn board_access<'a>(access: &'a BoardAccess, ticket: &Ticket) -> &'a [ActorAccess] {
    access.get(&ticket.issue.repo).map_or(&[], Vec::as_slice)
}

/// One line per ticket: issue, state, labels, title, in columns.
fn print_list(tickets: &[Ticket]) {
    let issues: Vec<String> = tickets.iter().map(|t| t.issue.to_string()).collect();
    let labels: Vec<String> = tickets
        .iter()
        .map(|t| {
            if t.labels.is_empty() {
                "-".to_owned()
            } else {
                t.labels.join(",")
            }
        })
        .collect();
    let issue_width = issues.iter().map(String::len).max().unwrap_or(0);
    let label_width = labels.iter().map(String::len).max().unwrap_or(0);
    for ((ticket, issue), labels) in tickets.iter().zip(&issues).zip(&labels) {
        let state = match ticket.state {
            IssueState::Open => "open",
            IssueState::Closed => "closed",
        };
        println!(
            "{issue:<issue_width$}  {state:<6}  {labels:<label_width$}  {}",
            ticket.title
        );
    }
}

/// Per ticket its verdict, and below it each rule that failed or was not
/// checked, with why; then the count of each verdict.
fn print_qualify(tickets: &[Ticket], access: &BoardAccess) {
    let issue_width = tickets
        .iter()
        .map(|t| t.issue.to_string().len())
        .max()
        .unwrap_or(0);
    let (mut qualified, mut refused, mut undecided) = (0, 0, 0);
    for ticket in tickets {
        let qualification = ::engine::qualify::qualify(ticket, board_access(access, ticket));
        let verdict = qualification.verdict();
        match verdict {
            Verdict::Qualified => qualified += 1,
            Verdict::Refused => refused += 1,
            Verdict::Undecided => undecided += 1,
        }
        println!(
            "{:<issue_width$}  {:<9}  {}",
            ticket.issue.to_string(),
            verdict.to_string(),
            ticket.title
        );
        for finding in &qualification.findings {
            let (how, why) = match &finding.outcome {
                Outcome::Pass => continue,
                Outcome::Fail(why) => ("failed", why),
                Outcome::NotChecked(why) => ("not checked", why),
            };
            println!("    {how:<11}  {}: {why}", finding.rule);
        }
    }
    if !tickets.is_empty() {
        println!("{qualified} qualified, {refused} refused, {undecided} undecided");
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    /// The words a POSIX shell makes of `line`, without its leading `kai`
    /// and its trailing `-- <command>`.
    fn words(line: &[u8]) -> Vec<Vec<u8>> {
        let line = line.strip_prefix(b"kai ").unwrap();
        let line = line.strip_suffix(b" -- <command>").unwrap();
        let mut script = b"set -- ".to_vec();
        script.extend_from_slice(line);
        script.extend_from_slice(b"\nprintf '%s\\0' \"$@\"");
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(OsStr::from_bytes(&script))
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let mut words: Vec<Vec<u8>> = output
            .stdout
            .split(|byte| *byte == 0)
            .map(<[u8]>::to_vec)
            .collect();
        assert_eq!(words.pop(), Some(Vec::new()));
        words
    }

    #[test]
    fn the_run_line_names_the_cache_and_keeps_each_path_one_word() {
        let cache = Path::new("/tmp/kai's cache/with spaces $HOME `id` ;&|<>*?[]{}\\\"\n");
        let fetch = cache.join("layers/a b.ext4");
        let build = Path::new(OsStr::from_bytes(b"/tmp/non-utf8-\xe9/it's.ext4"));
        let line = run_line("rust", cache, &fetch, build);
        assert!(line.starts_with(b"kai vm run --cache '"));
        let expected: Vec<Vec<u8>> = [
            b"vm".as_slice(),
            b"run",
            b"--cache",
            cache.as_os_str().as_bytes(),
            b"--image",
            b"rust",
            b"--layer",
            fetch.as_os_str().as_bytes(),
            b"--layer",
            build.as_os_str().as_bytes(),
            b"--cwd",
            b"/src",
        ]
        .iter()
        .map(|word| word.to_vec())
        .collect();
        assert_eq!(words(&line), expected);
    }

    /// Paths built to break out of their quotes, each with its own way to
    /// run `touch`: if quoting fails, a file `pwned-<n>` appears.
    const HOSTILE: [&[u8]; 9] = [
        b"/c/x'; touch pwned-1; echo '",
        b"/c/$(touch pwned-2)",
        b"/c/`touch pwned-3`",
        b"/c/x\"; touch pwned-4; \"",
        b"/c/x\\'; touch pwned-5; #",
        b"/c/x\ntouch pwned-6\n",
        b"/c/a''b'; touch pwned-7; '",
        b"/c/x!$HOME ~ * ? [a] {b,c} | & ; < > \t",
        b"/c/\xe9'\\''; touch pwned-9 #",
    ];

    /// The shells the line is run in: `sh` always; bash, dash and zsh, the
    /// last also with `RC_QUOTES`, where they are installed.
    fn shells() -> Vec<Vec<&'static str>> {
        let mut shells = vec![vec!["sh"]];
        for shell in [
            &["bash"][..],
            &["dash"],
            &["zsh"],
            &["zsh", "-o", "rcquotes"],
        ] {
            let there = std::process::Command::new(shell[0])
                .args(["-c", "true"])
                .output()
                .is_ok_and(|output| output.status.success());
            if there {
                shells.push(shell.to_vec());
            }
        }
        shells
    }

    /// The line run as a reader would paste it, `<command>` filled in with
    /// `true`, in an empty directory where `kai` is a shell function that
    /// only writes down its arguments: what comes back are the arguments,
    /// and the directory holds nothing else.
    #[test]
    fn a_copied_line_runs_nothing_but_kai_with_its_arguments_unchanged() {
        let dir = std::env::temp_dir().join(format!("kai-run-line-{}", std::process::id()));
        for shell in shells() {
            for (n, cache) in HOSTILE.iter().enumerate() {
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).unwrap();
                let cache = Path::new(OsStr::from_bytes(cache));
                let fetch = Path::new(OsStr::from_bytes(HOSTILE[(n + 1) % HOSTILE.len()]));
                let build = Path::new(OsStr::from_bytes(HOSTILE[(n + 2) % HOSTILE.len()]));
                let line = run_line("rust", cache, fetch, build);
                let line = line.strip_suffix(b"<command>").unwrap();
                let mut script = b"kai() { printf '%s\\0' \"$@\" > args; }\n".to_vec();
                script.extend_from_slice(line);
                script.extend_from_slice(b"true\n");
                let output = std::process::Command::new(shell[0])
                    .args(&shell[1..])
                    .arg("-c")
                    .arg(OsStr::from_bytes(&script))
                    .current_dir(&dir)
                    .env_clear()
                    .env("PATH", std::env::var_os("PATH").unwrap())
                    .output()
                    .unwrap();
                let what = format!("{shell:?}, cache #{n}");
                assert!(output.status.success(), "{what}: {output:?}");
                let mut names: Vec<String> = std::fs::read_dir(&dir)
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .collect();
                names.sort();
                assert_eq!(names, ["args"], "{what}");
                let args = std::fs::read(dir.join("args")).unwrap();
                let mut words: Vec<&[u8]> = args.split(|byte| *byte == 0).collect();
                assert_eq!(words.pop(), Some(&b""[..]), "{what}");
                let expected: [&[u8]; 14] = [
                    b"vm",
                    b"run",
                    b"--cache",
                    cache.as_os_str().as_bytes(),
                    b"--image",
                    b"rust",
                    b"--layer",
                    fetch.as_os_str().as_bytes(),
                    b"--layer",
                    build.as_os_str().as_bytes(),
                    b"--cwd",
                    b"/src",
                    b"--",
                    b"true",
                ];
                assert_eq!(words, expected[..].to_vec(), "{what}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What fish needs besides what POSIX shells need: no `'` and no `\`
    /// between single quotes. Checked here, since fish need not be
    /// installed where the tests run.
    #[test]
    fn no_apostrophe_and_no_backslash_stands_between_single_quotes() {
        assert_eq!(shell_word(b"it's"), b"'it'\"'\"'s'");
        assert_eq!(shell_word(b"a\\b"), b"'a'\"\\\\\"'b'");
        assert_eq!(shell_word(b"''"), b"\"'\"\"'\"");
        for word in HOSTILE {
            let quoted = shell_word(word);
            let mut single = false;
            let mut double = false;
            for byte in quoted {
                match byte {
                    b'\'' if !double => single = !single,
                    b'"' if !single => double = !double,
                    b'\\' if single => panic!("a backslash between single quotes"),
                    _ => {}
                }
            }
            assert!(!single && !double, "{}", String::from_utf8_lossy(word));
        }
    }

    #[test]
    fn a_plain_path_stays_as_it_is() {
        let line = run_line(
            "rust",
            Path::new("/cache/kai/vm"),
            Path::new("/cache/kai/vm/layers/a.ext4"),
            Path::new("/cache/kai/vm/layers/b.ext4"),
        );
        assert_eq!(
            String::from_utf8(line).unwrap(),
            "kai vm run --cache /cache/kai/vm --image rust \
             --layer /cache/kai/vm/layers/a.ext4 \
             --layer /cache/kai/vm/layers/b.ext4 --cwd /src -- <command>"
        );
        assert_eq!(shell_word(b""), b"''");
    }
}
