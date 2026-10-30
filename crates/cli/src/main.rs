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
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use surimono::{Error, ErrorPolicy, Limits, Options, Renderer, TextMode};

const HELP: &str = "\
surimono: run a document's JavaScript without a browser, so browserless
HTML-to-PDF engines can print it. In this version: chart islands.

Usage:
  surimono render <in.html> -o <out.html> [--font <file>]... [options]
  surimono --help | --version

  <in.html> and <out.html> may be - for standard input and output.
  -o defaults to standard output.

Options:
  --font <file>            A font for measuring text (TTF, OTF or TTC). Repeat it;
                           pass the fonts your PDF engine prints with. System
                           fonts are never used.
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

Exit codes:
  0  success
  1  an island failed: invalid marker, unknown renderer, script error, unusable SVG
  2  usage error: unknown option or bad value
  3  a script hit a limit (memory, time or stack)
  4  invalid configuration: a font that can't be used, limits out of range
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
        Some(path) => fs::write(path, output).map_err(|e| Failure::io(path, &e)),
    }
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
        ];
        for (e, exit) in cases {
            assert_eq!(Failure::from_error(&e).exit, exit, "{e}");
        }
    }

    /// An IslandRef from a real failed render, since it can't be built outside the facade.
    fn placeholder_ref() -> surimono::IslandRef {
        let r = Renderer::new(Options::new()).unwrap_or_else(|e| panic!("{e}"));
        match r.render_islands(
            "<script type=\"application/vnd.surimono+json\" data-renderer=\"none\">{}</script>",
        ) {
            Err(Error::UnknownRenderer { island, .. }) => island,
            other => panic!("{other:?}"),
        }
    }
}
