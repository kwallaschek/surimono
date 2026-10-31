//! The `surimono` command-line interface.
//!
//! ```text
//! surimono render <in.html> -o <out.html> --font <file>... [options]
//! ```
//!
//! `-` reads standard input or writes standard output. Exit codes are typed;
//! see [`Exit`] and `surimono --help`.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use surimono::{Error, ErrorPolicy, Limits, Options, Renderer, TextMode};

const HELP: &str = "\
surimono: run a document's JavaScript without a browser, so browserless
HTML-to-PDF engines can print it. In this version: chart islands.

Usage:
  surimono render <in.html> -o <out.html> --font <file>... [options]
  surimono --help | --version

  <in.html> and <out.html> may be - for standard input and output.
  -o defaults to standard output.

Options:
  --font <file>            Required, at least one: a font for measuring and
                           outlining text (TTF, OTF or TTC). Repeat it; pass the
                           fonts your PDF engine prints with. System fonts are
                           never used.
  --memory-mb <n>          JavaScript heap per island (default 64)
  --time-ms <n>            Time per island (default 2000)
  --stack-kb <n>           JavaScript stack per island (default 512)
  --seed <n>               Seed for Math.random (default 0)
  --now-ms <n>             What Date.now() returns, ms since 1970 (default
                           1767225600000, 2026-01-01T00:00:00Z)
  --utc-offset-minutes <n> Local time for scripts, e.g. 540 for Japan (default 0)
  --error-policy <p>       fail (default): stop at the first failed island;
                           placeholder: leave an empty, sized div instead
  --text <mode>            outlines (default): chart text becomes glyph outlines
                           in your fonts, so any PDF engine prints what was
                           measured; keep: leave <text> (needs an engine that
                           sets inline SVG text in your fonts)
  --bundle <name>=<file>   Add a renderer from your own JavaScript bundle: a
                           script defining a global function entry(specJson,
                           ctx) that returns SVG. Islands use it with
                           data-renderer=\"<name>\". Repeat it for more.
  --bundle <name>:<fn>=<file>
                           The same, calling the global function <fn>
  --renderers <a,b,...>    Enable only these renderers (built-in or bundles;
                           default: all)
  --max-islands <n>        Most islands one document may have (default 1000)
  --pool-size <n>          JavaScript runtimes kept (default: CPUs, at most 4)

  The output file is written only after the render succeeded, through a
  temporary file renamed into place, so it is either complete or untouched.

Exit codes:
  0  success
  1  an island failed (invalid marker, unknown renderer, script error, unusable
     SVG), the document has more islands than --max-islands, or the document
     couldn't be rewritten
  2  usage error: unknown option or bad value
  3  a script hit a limit (memory, time or stack)
  4  invalid configuration: no font, a font or bundle that can't be used, limits
     out of range
  5  a file couldn't be read or written
";

/// What the process exits with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Exit {
    Ok = 0,
    Island = 1,
    Usage = 2,
    Limit = 3,
    Config = 4,
    Io = 5,
}

/// A failure, with the exit code it maps to.
#[derive(Debug)]
struct Failure {
    exit: Exit,
    message: String,
}

impl Failure {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            exit: Exit::Usage,
            message: format!("{}\n(see surimono --help)", message.into()),
        }
    }

    fn io(what: &str, e: &io::Error) -> Self {
        Self {
            exit: Exit::Io,
            message: format!("{what}: {e}"),
        }
    }

    fn from_error(e: &Error) -> Self {
        let exit = match e {
            Error::MemoryLimit { .. } | Error::TimeLimit { .. } | Error::StackOverflow { .. } => {
                Exit::Limit
            }
            Error::Font { .. } | Error::Bundle { .. } | Error::InvalidOptions(_) => Exit::Config,
            _ => Exit::Island,
        };
        Self {
            exit,
            message: format!("{e} [{}]", e.kind()),
        }
    }
}

/// A `--bundle` argument: renderer name, entry function, script file.
#[derive(Debug, PartialEq, Eq)]
struct BundleArg {
    name: String,
    entry: String,
    path: PathBuf,
}

/// The parsed `render` command.
#[derive(Debug, Default)]
struct RenderArgs {
    input: Option<String>,
    output: Option<String>,
    fonts: Vec<PathBuf>,
    limits: Limits,
    seed: u64,
    now_ms: Option<i64>,
    utc_offset_minutes: i32,
    policy: ErrorPolicy,
    text: TextMode,
    bundles: Vec<BundleArg>,
    renderers: Option<Vec<String>>,
    max_islands: Option<usize>,
    pool_size: Option<usize>,
}

enum Command {
    Help,
    Version,
    Render(Box<RenderArgs>),
}

fn parse(args: Vec<OsString>) -> Result<Command, Failure> {
    let mut args = args.into_iter().map(|a| {
        a.into_string()
            .map_err(|a| Failure::usage(format!("argument {a:?} is not valid UTF-8")))
    });
    let first = match args.next().transpose()? {
        None => return Err(Failure::usage("no command given")),
        Some(a) => a,
    };
    match first.as_str() {
        "-h" | "--help" | "help" => return Ok(Command::Help),
        "-V" | "--version" => return Ok(Command::Version),
        "render" => {}
        other => return Err(Failure::usage(format!("unknown command {other:?}"))),
    }
    let mut r = RenderArgs::default();
    let mut only_positional = false;
    while let Some(arg) = args.next().transpose()? {
        if only_positional || arg == "-" || !arg.starts_with('-') {
            if r.input.replace(arg.clone()).is_some() {
                return Err(Failure::usage(format!("unexpected argument {arg:?}")));
            }
            continue;
        }
        if arg == "--" {
            only_positional = true;
            continue;
        }
        if arg == "-h" || arg == "--help" {
            return Ok(Command::Help);
        }
        // --flag value or --flag=value.
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_owned(), Some(v.to_owned())),
            _ => (arg.clone(), None),
        };
        let mut value = || -> Result<String, Failure> {
            match inline.clone() {
                Some(v) => Ok(v),
                None => args
                    .next()
                    .transpose()?
                    .ok_or_else(|| Failure::usage(format!("{flag} needs a value"))),
            }
        };
        match flag.as_str() {
            "-o" | "--output" => r.output = Some(value()?),
            "--font" => r.fonts.push(PathBuf::from(value()?)),
            "--memory-mb" => {
                r.limits.memory_bytes = mib(number::<usize>(&flag, &value()?)?, &flag)?;
            }
            "--time-ms" => {
                r.limits.time = Duration::from_millis(number(&flag, &value()?)?);
            }
            "--stack-kb" => {
                let kb: usize = number(&flag, &value()?)?;
                r.limits.stack_bytes = kb
                    .checked_mul(1024)
                    .ok_or_else(|| Failure::usage(format!("{flag} is too large")))?;
            }
            "--seed" => r.seed = number(&flag, &value()?)?,
            "--now-ms" => r.now_ms = Some(number(&flag, &value()?)?),
            "--utc-offset-minutes" => r.utc_offset_minutes = number(&flag, &value()?)?,
            "--error-policy" => {
                r.policy = match value()?.as_str() {
                    "fail" => ErrorPolicy::Fail,
                    "placeholder" => ErrorPolicy::Placeholder,
                    other => {
                        return Err(Failure::usage(format!(
                            "--error-policy must be fail or placeholder, got {other:?}"
                        )));
                    }
                }
            }
            "--text" => {
                r.text = match value()?.as_str() {
                    "outlines" => TextMode::Outlines,
                    "keep" => TextMode::Keep,
                    other => {
                        return Err(Failure::usage(format!(
                            "--text must be outlines or keep, got {other:?}"
                        )));
                    }
                }
            }
            "--bundle" => r.bundles.push(bundle_arg(&value()?)?),
            "--renderers" => {
                let list = value()?;
                let names: Vec<String> = list
                    .split(',')
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .map(str::to_owned)
                    .collect();
                if names.is_empty() {
                    return Err(Failure::usage(
                        "--renderers needs at least one name, e.g. --renderers echarts,mine",
                    ));
                }
                r.renderers = Some(names);
            }
            "--max-islands" => r.max_islands = Some(number(&flag, &value()?)?),
            "--pool-size" => r.pool_size = Some(number(&flag, &value()?)?),
            other => return Err(Failure::usage(format!("unknown option {other:?}"))),
        }
    }
    if r.input.is_none() {
        return Err(Failure::usage(
            "render needs an input file (or - for stdin)",
        ));
    }
    Ok(Command::Render(Box::new(r)))
}

/// `<name>=<file>` or `<name>:<entry>=<file>`.
fn bundle_arg(value: &str) -> Result<BundleArg, Failure> {
    let Some((head, path)) = value.split_once('=') else {
        return Err(Failure::usage(format!(
            "--bundle needs <name>=<file> or <name>:<function>=<file>, got {value:?}"
        )));
    };
    let (name, entry) = head.split_once(':').unwrap_or((head, "entry"));
    if name.is_empty() || entry.is_empty() || path.is_empty() {
        return Err(Failure::usage(format!(
            "--bundle needs <name>=<file> or <name>:<function>=<file>, got {value:?}"
        )));
    }
    Ok(BundleArg {
        name: name.to_owned(),
        entry: entry.to_owned(),
        path: PathBuf::from(path),
    })
}

fn number<T: std::str::FromStr>(flag: &str, value: &str) -> Result<T, Failure> {
    value
        .parse()
        .map_err(|_| Failure::usage(format!("{flag} needs a whole number, got {value:?}")))
}

fn mib(n: usize, flag: &str) -> Result<usize, Failure> {
    n.checked_mul(1024 * 1024)
        .ok_or_else(|| Failure::usage(format!("{flag} is too large")))
}

fn render(args: &RenderArgs) -> Result<(), Failure> {
    let mut options = Options::new()
        .limits(args.limits)
        .seed(args.seed)
        .utc_offset_minutes(args.utc_offset_minutes)
        .error_policy(args.policy)
        .text_mode(args.text);
    if let Some(ms) = args.now_ms {
        options = options.now_ms(ms);
    }
    for path in &args.fonts {
        let bytes = fs::read(path).map_err(|e| Failure::io(&path.display().to_string(), &e))?;
        options = options.font(bytes);
    }
    for b in &args.bundles {
        let source = fs::read_to_string(&b.path)
            .map_err(|e| Failure::io(&b.path.display().to_string(), &e))?;
        options = options.bundle(&b.name, source, &b.entry);
    }
    if let Some(names) = &args.renderers {
        options = options.renderers(names);
    }
    if let Some(max) = args.max_islands {
        options = options.max_islands(max);
    }
    if let Some(size) = args.pool_size {
        options = options.pool_size(size);
    }
    let input = match args.input.as_deref() {
        None | Some("-") => {
            let mut s = String::new();
            io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| Failure::io("standard input", &e))?;
            s
        }
        Some(path) => fs::read_to_string(path).map_err(|e| Failure::io(path, &e))?,
    };
    let renderer = Renderer::new(options).map_err(|e| Failure::from_error(&e))?;
    let output = renderer
        .render_islands(&input)
        .map_err(|e| Failure::from_error(&e))?;
    // Written only after the render succeeded, so a failure leaves no file.
    match args.output.as_deref() {
        None | Some("-") => {
            let mut out = io::stdout().lock();
            out.write_all(output.as_bytes())
                .and_then(|()| out.flush())
                .map_err(|e| Failure::io("standard output", &e))
        }
        Some(path) => {
            write_atomically(Path::new(path), output.as_bytes()).map_err(|e| Failure::io(path, &e))
        }
    }
}

/// Writes `bytes` to `path` through a temporary file in the same directory,
/// synced and then renamed over `path`, so `path` ends up either complete or
/// as it was (a full disk or a quota leaves no truncated file). The
/// temporary file is removed on any failure.
fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a file name"))?;
    let mut attempt = 0u32;
    let (tmp, mut file) = loop {
        let mut tmp_name = OsString::from(".");
        tmp_name.push(name);
        tmp_name.push(format!(".surimono-{}-{attempt}.tmp", std::process::id()));
        let tmp = dir.join(tmp_name);
        match fs::File::create_new(&tmp) {
            Ok(f) => break (tmp, f),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && attempt < 100 => attempt += 1,
            Err(e) => return Err(e),
        }
    };
    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| {
            drop(file);
            fs::rename(&tmp, path)
        });
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

fn main() -> ExitCode {
    let outcome = parse(std::env::args_os().skip(1).collect()).and_then(|command| match command {
        Command::Help => {
            print!("{HELP}");
            Ok(())
        }
        Command::Version => {
            println!("surimono {}", surimono::VERSION);
            Ok(())
        }
        Command::Render(args) => render(&args),
    });
    match outcome {
        Ok(()) => ExitCode::from(Exit::Ok as u8),
        Err(f) => {
            eprintln!("surimono: error: {}", f.message);
            ExitCode::from(f.exit as u8)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    fn render_args(list: &[&str]) -> RenderArgs {
        match parse(args(list)) {
            Ok(Command::Render(r)) => *r,
            Ok(_) => panic!("not a render command"),
            Err(f) => panic!("{}", f.message),
        }
    }

    #[test]
    fn parses_every_option() {
        let r = render_args(&[
            "render",
            "in.html",
            "-o",
            "out.html",
            "--font",
            "a.ttf",
            "--font=b.otf",
            "--memory-mb",
            "8",
            "--time-ms=500",
            "--stack-kb",
            "256",
            "--seed",
            "7",
            "--now-ms",
            "-5",
            "--utc-offset-minutes",
            "540",
            "--error-policy",
            "placeholder",
            "--text",
            "keep",
            "--bundle",
            "mine=a.js",
            "--bundle=other:draw=b/c.js",
            "--renderers",
            "echarts, mine",
            "--max-islands",
            "20",
            "--pool-size=2",
        ]);
        assert_eq!(r.input.as_deref(), Some("in.html"));
        assert_eq!(r.output.as_deref(), Some("out.html"));
        assert_eq!(
            r.fonts,
            vec![PathBuf::from("a.ttf"), PathBuf::from("b.otf")]
        );
        assert_eq!(r.limits.memory_bytes, 8 * 1024 * 1024);
        assert_eq!(r.limits.time, Duration::from_millis(500));
        assert_eq!(r.limits.stack_bytes, 256 * 1024);
        assert_eq!((r.seed, r.now_ms, r.utc_offset_minutes), (7, Some(-5), 540));
        assert_eq!(r.policy, ErrorPolicy::Placeholder);
        assert_eq!(r.text, TextMode::Keep);
        assert_eq!(
            r.bundles,
            vec![
                BundleArg {
                    name: "mine".into(),
                    entry: "entry".into(),
                    path: PathBuf::from("a.js")
                },
                BundleArg {
                    name: "other".into(),
                    entry: "draw".into(),
                    path: PathBuf::from("b/c.js")
                },
            ]
        );
        assert_eq!(
            r.renderers,
            Some(vec!["echarts".to_owned(), "mine".to_owned()])
        );
        assert_eq!((r.max_islands, r.pool_size), (Some(20), Some(2)));
    }

    #[test]
    fn dash_and_defaults() {
        let r = render_args(&["render", "-"]);
        assert_eq!(r.input.as_deref(), Some("-"));
        assert_eq!(r.output, None);
        assert_eq!(r.limits, Limits::default());
        assert_eq!(r.policy, ErrorPolicy::Fail);
        let r = render_args(&["render", "--", "-o"]);
        assert_eq!(r.input.as_deref(), Some("-o"));
    }

    #[test]
    fn usage_errors() {
        for bad in [
            &[][..],
            &["draw"],
            &["render"],
            &["render", "a", "b"],
            &["render", "a", "--font"],
            &["render", "a", "--time-ms", "soon"],
            &["render", "a", "--memory-mb", "-1"],
            &["render", "a", "--memory-mb", "99999999999999999"],
            &["render", "a", "--error-policy", "ignore"],
            &["render", "a", "--colour"],
            &["render", "a", "--text", "pixels"],
            &["render", "a", "--bundle", "mine"],
            &["render", "a", "--bundle", "=a.js"],
            &["render", "a", "--bundle", "mine:=a.js"],
            &["render", "a", "--bundle", "mine="],
            &["render", "a", "--renderers", ","],
            &["render", "a", "--max-islands", "many"],
            &["render", "a", "--pool-size", "-2"],
        ] {
            match parse(args(bad)) {
                Err(f) => assert_eq!(f.exit, Exit::Usage, "{bad:?}"),
                Ok(_) => panic!("{bad:?} parsed"),
            }
        }
        assert!(matches!(parse(args(&["--help"])), Ok(Command::Help)));
        assert!(matches!(parse(args(&["render", "-h"])), Ok(Command::Help)));
        assert!(matches!(parse(args(&["-V"])), Ok(Command::Version)));
    }

    #[test]
    fn errors_map_to_exit_codes() {
        let island = || surimono::IslandRef::clone(&placeholder_ref());
        let cases = [
            (Error::TimeLimit { island: island() }, Exit::Limit),
            (Error::MemoryLimit { island: island() }, Exit::Limit),
            (Error::InvalidOptions("x".into()), Exit::Config),
            (Error::Html("x".into()), Exit::Island),
            (Error::TooManyIslands { max: 3 }, Exit::Island),
            (
                Error::Bundle {
                    name: "b".into(),
                    message: "x".into(),
                },
                Exit::Config,
            ),
        ];
        for (e, exit) in cases {
            assert_eq!(Failure::from_error(&e).exit, exit, "{e}");
        }
    }

    #[test]
    fn atomic_write_replaces_or_leaves_alone() {
        let dir = std::env::temp_dir().join(format!("surimono-cli-unit-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let out = dir.join("out.html");
        fs::write(&out, "old").unwrap_or_else(|e| panic!("{e}"));
        write_atomically(&out, b"new").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fs::read_to_string(&out).ok().as_deref(), Some("new"));
        // A target that can't be replaced (a directory) fails, leaves it as it
        // was and leaves no temporary file behind.
        let blocked = dir.join("blocked");
        fs::create_dir_all(blocked.join("inside")).unwrap_or_else(|e| panic!("{e}"));
        assert!(write_atomically(&blocked, b"x").is_err());
        assert!(blocked.join("inside").is_dir());
        let left: Vec<_> = fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{e}"))
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// An IslandRef from a real failed render, since it can't be built outside the facade.
    fn placeholder_ref() -> surimono::IslandRef {
        let font = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/fonts/NotoSans-Regular.ttf"
        ))
        .unwrap_or_else(|e| panic!("{e}"));
        let r = Renderer::new(Options::new().font(font)).unwrap_or_else(|e| panic!("{e}"));
        match r.render_islands(
            "<script type=\"application/vnd.surimono+json\" data-renderer=\"none\">{}</script>",
        ) {
            Err(Error::UnknownRenderer { island, .. }) => island,
            other => panic!("{other:?}"),
        }
    }
}
