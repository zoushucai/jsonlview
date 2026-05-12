use std::collections::VecDeque;
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process;

use anyhow::Result;
use jlv::{
    OutputOptions, attach_tail_line_numbers, count_lines, read_head, read_random_buffered,
    read_random_fast, read_range, read_tail, render_lines,
};

const DEFAULT_NUM: usize = 5;
const DEFAULT_START: usize = 0;
const DEFAULT_MAX_CHARS: usize = 60;

const HELP: &str = "\
jlv

Usage:
  jlv <command> [command options] [global options] <FILE>
  jlv -c [--file FILE | FILE]

Commands:
  head                View the first lines
  tail                View the last lines
  range               View a range starting from a zero-based offset
  random              View random lines quickly with approximate sampling
  random-fast         Alias of random
  random-buf          View random lines using full-file reservoir sampling

Global options:
  -f, --file <FILE>   Input JSONL file; can also be given as a bare path
  -p, --pretty [NUM]  Pretty-print JSON; NUM controls blank lines between entries
  -m, --max <NUM>     Truncate long string fields when pretty-printing [default: 60]
  -l, --line          Show [line N] prefixes
  -c, --count         Count total lines in the file
  -h, --help          Show help
  -V, --version       Show version

Command options:
  head/tail/random/random-fast/random-buf:
    -n, --num <NUM>   Number of lines to show [default: 5]
    Also supports positional form: <command> [num] <FILE>

  range:
    -s, --start <N>   Zero-based start offset [default: 0]
    -n, --num <NUM>   Number of lines to show [default: 5]
    Also supports positional form: range [start] [num]

Examples:
  jlv head -n 5 \"example\\data.jsonl\"
  jlv head 5 \"example\\data.jsonl\"
  jlv head \"example\\data.jsonl\" -p
  jlv tail \"example\\data.jsonl\"
  jlv range 10 20 --pretty 0 \"example\\data.jsonl\"
  jlv range --start 10 --num 20 -p 1 -l \"example\\data.jsonl\"
  jlv random 3 \"example\\data.jsonl\"
  jlv random-buf -n 3 -m 40 -p 2 \"example\\data.jsonl\"
  jlv --count \"example\\data.jsonl\"
";

fn main() {
    match parse_cli(env::args_os()) {
        Ok(ParseOutcome::Help) => {
            print!("{HELP}");
        }
        Ok(ParseOutcome::Version) => {
            println!("jlv {}", env!("CARGO_PKG_VERSION"));
        }
        Ok(ParseOutcome::Run(cli)) => {
            if let Err(err) = run(cli) {
                eprintln!("error: {err}");
                process::exit(1);
            }
        }
        Err(err) => {
            eprintln!("error: {err}\n");
            eprintln!("{HELP}");
            process::exit(2);
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.action {
        Action::CountLines => {
            println!("{}", count_lines(&cli.file)?);
            Ok(())
        }
        Action::Head { num } => {
            let entries = read_head(&cli.file, num)?;
            print!("{}", render_lines(&entries, &cli.output)?);
            Ok(())
        }
        Action::Tail { num } => {
            let mut entries = read_tail(&cli.file, num)?;
            if cli.output.show_line_numbers {
                attach_tail_line_numbers(&cli.file, &mut entries)?;
            }
            print!("{}", render_lines(&entries, &cli.output)?);
            Ok(())
        }
        Action::Range { start, num } => {
            let entries = read_range(&cli.file, start, num)?;
            print!("{}", render_lines(&entries, &cli.output)?);
            Ok(())
        }
        Action::Random { num } => {
            let entries = read_random_fast(&cli.file, num)?;
            print!("{}", render_lines(&entries, &cli.output)?);
            Ok(())
        }
        Action::RandomBuffered { num } => {
            let entries = read_random_buffered(&cli.file, num)?;
            print!("{}", render_lines(&entries, &cli.output)?);
            Ok(())
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Cli {
    file: PathBuf,
    output: OutputOptions,
    action: Action,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    CountLines,
    Head { num: usize },
    Tail { num: usize },
    Range { start: usize, num: usize },
    Random { num: usize },
    RandomBuffered { num: usize },
}

#[derive(Debug, PartialEq, Eq)]
enum ParseOutcome {
    Help,
    Version,
    Run(Cli),
}

#[derive(Default)]
struct RawArgs {
    file: Option<PathBuf>,
    pretty: Option<usize>,
    max_chars: usize,
    show_line_numbers: bool,
    count_lines: bool,
    command: Option<CommandName>,
    num: Option<usize>,
    start: Option<usize>,
    positionals: Vec<OsString>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandName {
    Head,
    Tail,
    Range,
    Random,
    RandomBuffered,
}

impl RawArgs {
    fn new() -> Self {
        Self {
            max_chars: DEFAULT_MAX_CHARS,
            ..Self::default()
        }
    }
}

fn parse_cli<I>(args: I) -> Result<ParseOutcome, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut tokens: VecDeque<OsString> = args.into_iter().collect();
    let _program = tokens.pop_front();
    if tokens.is_empty() {
        return Ok(ParseOutcome::Help);
    }

    let mut raw = RawArgs::new();

    while let Some(token) = tokens.pop_front() {
        let text = token.to_string_lossy();
        match text.as_ref() {
            "-h" | "--help" => return Ok(ParseOutcome::Help),
            "-V" | "--version" => return Ok(ParseOutcome::Version),
            "-p" | "--pretty" => {
                raw.pretty = Some(parse_optional_pretty_gap(&mut tokens)?);
            }
            "-l" | "--line" => raw.show_line_numbers = true,
            "-c" | "--count" => raw.count_lines = true,
            "-f" | "--file" => raw.file = Some(PathBuf::from(next_value(&mut tokens, "--file")?)),
            "-m" | "--max" => {
                raw.max_chars = parse_usize(next_value(&mut tokens, "--max")?, "--max")?;
            }
            "-n" | "--num" => {
                raw.num = Some(parse_usize(next_value(&mut tokens, "--num")?, "--num")?);
            }
            "-s" | "--start" => {
                raw.start = Some(parse_usize(next_value(&mut tokens, "--start")?, "--start")?);
            }
            "head" => set_command(&mut raw, CommandName::Head)?,
            "tail" => set_command(&mut raw, CommandName::Tail)?,
            "range" => set_command(&mut raw, CommandName::Range)?,
            "random" | "random-fast" => set_command(&mut raw, CommandName::Random)?,
            "random-buf" => set_command(&mut raw, CommandName::RandomBuffered)?,
            _ if text.starts_with("--file=") => {
                raw.file = Some(PathBuf::from(value_after_equals(&text, "--file=")?));
            }
            _ if text.starts_with("--pretty=") => {
                raw.pretty = Some(parse_pretty_gap(value_after_equals(&text, "--pretty=")?)?);
            }
            _ if text.starts_with("--max=") => {
                raw.max_chars = parse_usize(value_after_equals(&text, "--max=")?, "--max")?;
            }
            _ if text.starts_with("--num=") => {
                raw.num = Some(parse_usize(value_after_equals(&text, "--num=")?, "--num")?);
            }
            _ if text.starts_with("--start=") => {
                raw.start = Some(parse_usize(value_after_equals(&text, "--start=")?, "--start")?);
            }
            _ if text.starts_with('-') => {
                return Err(format!("unknown option: {}", text));
            }
            _ => raw.positionals.push(token),
        }
    }

    finalize_args(raw)
}

fn finalize_args(raw: RawArgs) -> Result<ParseOutcome, String> {
    if raw.count_lines {
        return finalize_count_mode(raw);
    }

    let command = raw
        .command
        .ok_or_else(|| "missing command; use head, tail, range, random, or -c/--count".to_owned())?;

    let (file, numeric_positionals) = split_file_and_numeric(raw.file, raw.positionals)?;
    let output = OutputOptions {
        pretty: raw.pretty,
        max_chars: raw.max_chars,
        show_line_numbers: raw.show_line_numbers,
    };

    let action = match command {
        CommandName::Head => {
            ensure_option_not_used("head", "--start", raw.start)?;
            Action::Head {
                num: resolve_num("head", raw.num, &numeric_positionals)?,
            }
        }
        CommandName::Tail => {
            ensure_option_not_used("tail", "--start", raw.start)?;
            Action::Tail {
                num: resolve_num("tail", raw.num, &numeric_positionals)?,
            }
        }
        CommandName::Random => {
            ensure_option_not_used("random", "--start", raw.start)?;
            Action::Random {
                num: resolve_num("random", raw.num, &numeric_positionals)?,
            }
        }
        CommandName::RandomBuffered => {
            ensure_option_not_used("random-buf", "--start", raw.start)?;
            Action::RandomBuffered {
                num: resolve_num("random-buf", raw.num, &numeric_positionals)?,
            }
        }
        CommandName::Range => {
            let mut numeric_positionals = numeric_positionals;
            let start = raw
                .start
                .unwrap_or_else(|| numeric_positionals.pop_front().unwrap_or(DEFAULT_START));
            let num = positive_or_default(
                raw.num.or_else(|| numeric_positionals.pop_front()),
                DEFAULT_NUM,
                "--num",
            )?;
            if let Some(extra) = numeric_positionals.pop_front() {
                return Err(format!("unexpected extra numeric value for range: {extra}"));
            }
            Action::Range { start, num }
        }
    };

    Ok(ParseOutcome::Run(Cli {
        file,
        output,
        action,
    }))
}

fn finalize_count_mode(raw: RawArgs) -> Result<ParseOutcome, String> {
    if raw.command.is_some() {
        return Err("`-c/--count` cannot be combined with head/tail/range/random".to_owned());
    }
    if raw.num.is_some() || raw.start.is_some() {
        return Err("`-c/--count` does not use `-n/--num` or `-s/--start`".to_owned());
    }
    if raw.pretty.is_some() || raw.show_line_numbers || raw.max_chars != DEFAULT_MAX_CHARS {
        return Err("`-c/--count` does not use `-p/--pretty`, `-l/--line`, or `-m/--max`".to_owned());
    }

    let (file, mut numeric_positionals) = split_file_and_numeric(raw.file, raw.positionals)?;
    if let Some(extra) = numeric_positionals.pop_front() {
        return Err(format!("unexpected numeric value in count mode: {extra}"));
    }

    Ok(ParseOutcome::Run(Cli {
        file,
        output: OutputOptions {
            pretty: None,
            max_chars: DEFAULT_MAX_CHARS,
            show_line_numbers: false,
        },
        action: Action::CountLines,
    }))
}

fn split_file_and_numeric(
    explicit_file: Option<PathBuf>,
    positionals: Vec<OsString>,
) -> Result<(PathBuf, VecDeque<usize>), String> {
    let mut file = explicit_file;
    let mut numeric = VecDeque::new();

    for positional in positionals {
        let text = positional.to_string_lossy();
        if let Ok(value) = text.parse::<usize>() {
            numeric.push_back(value);
            continue;
        }

        if file.is_some() {
            return Err(format!(
                "multiple file paths provided; unexpected extra value: {}",
                positional.to_string_lossy()
            ));
        }
        file = Some(PathBuf::from(positional));
    }

    let file = file.ok_or_else(|| "missing input file; use `-f/--file` or provide a bare file path".to_owned())?;
    Ok((file, numeric))
}

fn set_command(raw: &mut RawArgs, command: CommandName) -> Result<(), String> {
    if let Some(existing) = raw.command {
        return Err(format!(
            "multiple commands provided: {:?} and {:?}",
            existing, command
        ));
    }
    raw.command = Some(command);
    Ok(())
}

fn ensure_option_not_used<T>(command: &str, option: &str, value: Option<T>) -> Result<(), String> {
    if value.is_some() {
        return Err(format!("`{option}` is not valid for `{command}`"));
    }
    Ok(())
}

fn resolve_num(command: &str, explicit: Option<usize>, values: &VecDeque<usize>) -> Result<usize, String> {
    if explicit.is_some() && !values.is_empty() {
        return Err(format!(
            "`{command}` received `num` more than once; use either positional `num` or `-n/--num`"
        ));
    }

    if values.len() > 1 {
        return Err(format!(
            "unexpected extra numeric value for `{command}`: {}",
            values[1]
        ));
    }

    positive_or_default(explicit.or_else(|| values.front().copied()), DEFAULT_NUM, "--num")
}

fn positive_or_default(value: Option<usize>, default: usize, option_name: &str) -> Result<usize, String> {
    let resolved = value.unwrap_or(default);
    if resolved == 0 {
        return Err(format!("`{option_name}` must be greater than 0"));
    }
    Ok(resolved)
}

fn next_value(tokens: &mut VecDeque<OsString>, option_name: &str) -> Result<OsString, String> {
    tokens
        .pop_front()
        .ok_or_else(|| format!("missing value for `{option_name}`"))
}

fn parse_usize(value: OsString, option_name: &str) -> Result<usize, String> {
    let text = value.to_string_lossy();
    text.parse::<usize>()
        .map_err(|_| format!("invalid value for `{option_name}`: {text}"))
}

fn parse_optional_pretty_gap(tokens: &mut VecDeque<OsString>) -> Result<usize, String> {
    match tokens.front() {
        Some(next) => {
            let text = next.to_string_lossy();
            match text.parse::<usize>() {
                Ok(value) => {
                    tokens.pop_front();
                    Ok(value)
                }
                Err(_) => Ok(0),
            }
        }
        None => Ok(0),
    }
}

fn parse_pretty_gap(value: OsString) -> Result<usize, String> {
    let text = value.to_string_lossy();
    text.parse::<usize>()
        .map_err(|_| format!("invalid value for `-p/--pretty`: {text}"))
}

fn value_after_equals<'a>(text: &'a str, prefix: &str) -> Result<OsString, String> {
    text.strip_prefix(prefix)
        .map(OsString::from)
        .ok_or_else(|| format!("invalid option syntax: {text}"))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{Action, Cli, DEFAULT_MAX_CHARS, DEFAULT_NUM, OutputOptions, ParseOutcome, parse_cli};

    #[test]
    fn head_uses_default_num_and_positional_file() {
        let parsed = parse_cli([
            OsString::from("jlv"),
            OsString::from("head"),
            OsString::from("sample.jsonl"),
        ])
        .unwrap();

        assert_eq!(
            parsed,
            ParseOutcome::Run(Cli {
                file: PathBuf::from("sample.jsonl"),
                output: OutputOptions {
                    pretty: None,
                    max_chars: DEFAULT_MAX_CHARS,
                    show_line_numbers: false,
                },
                action: Action::Head { num: DEFAULT_NUM },
            })
        );
    }

    #[test]
    fn range_accepts_mixed_order_and_flags() {
        let parsed = parse_cli([
            OsString::from("jlv"),
            OsString::from("--file"),
            OsString::from("sample.jsonl"),
            OsString::from("range"),
            OsString::from("--pretty"),
            OsString::from("1"),
            OsString::from("10"),
            OsString::from("-n"),
            OsString::from("20"),
            OsString::from("-l"),
        ])
        .unwrap();

        assert_eq!(
            parsed,
            ParseOutcome::Run(Cli {
                file: PathBuf::from("sample.jsonl"),
                output: OutputOptions {
                    pretty: Some(1),
                    max_chars: DEFAULT_MAX_CHARS,
                    show_line_numbers: true,
                },
                action: Action::Range { start: 10, num: 20 },
            })
        );
    }

    #[test]
    fn count_mode_requires_no_command() {
        let err = parse_cli([
            OsString::from("jlv"),
            OsString::from("-c"),
            OsString::from("head"),
            OsString::from("sample.jsonl"),
        ])
        .unwrap_err();

        assert!(err.contains("cannot be combined"));
    }

    #[test]
    fn head_accepts_positional_num_before_file() {
        let parsed = parse_cli([
            OsString::from("jlv"),
            OsString::from("head"),
            OsString::from("5"),
            OsString::from("sample.jsonl"),
        ])
        .unwrap();

        assert_eq!(
            parsed,
            ParseOutcome::Run(Cli {
                file: PathBuf::from("sample.jsonl"),
                output: OutputOptions {
                    pretty: None,
                    max_chars: DEFAULT_MAX_CHARS,
                    show_line_numbers: false,
                },
                action: Action::Head { num: 5 },
            })
        );
    }

    #[test]
    fn pretty_without_value_defaults_to_zero() {
        let parsed = parse_cli([
            OsString::from("jlv"),
            OsString::from("head"),
            OsString::from("sample.jsonl"),
            OsString::from("-p"),
        ])
        .unwrap();

        assert_eq!(
            parsed,
            ParseOutcome::Run(Cli {
                file: PathBuf::from("sample.jsonl"),
                output: OutputOptions {
                    pretty: Some(0),
                    max_chars: DEFAULT_MAX_CHARS,
                    show_line_numbers: false,
                },
                action: Action::Head { num: DEFAULT_NUM },
            })
        );
    }

    #[test]
    fn pretty_accepts_any_non_negative_integer() {
        let parsed = parse_cli([
            OsString::from("jlv"),
            OsString::from("random"),
            OsString::from("-p"),
            OsString::from("3"),
            OsString::from("sample.jsonl"),
        ])
        .unwrap();

        assert_eq!(
            parsed,
            ParseOutcome::Run(Cli {
                file: PathBuf::from("sample.jsonl"),
                output: OutputOptions {
                    pretty: Some(3),
                    max_chars: DEFAULT_MAX_CHARS,
                    show_line_numbers: false,
                },
                action: Action::Random { num: DEFAULT_NUM },
            })
        );
    }

    #[test]
    fn random_buf_command_is_supported() {
        let parsed = parse_cli([
            OsString::from("jlv"),
            OsString::from("random-buf"),
            OsString::from("7"),
            OsString::from("sample.jsonl"),
        ])
        .unwrap();

        assert_eq!(
            parsed,
            ParseOutcome::Run(Cli {
                file: PathBuf::from("sample.jsonl"),
                output: OutputOptions {
                    pretty: None,
                    max_chars: DEFAULT_MAX_CHARS,
                    show_line_numbers: false,
                },
                action: Action::RandomBuffered { num: 7 },
            })
        );
    }
}
