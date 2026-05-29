//! Classification of the `git commit` flags the user passed.
//!
//! Pure (no I/O): turns the raw `git_args` into a [`ParsedArgs`] describing how
//! to generate the message and what to forward to `git commit`. The two
//! predicates [`wants_help`] and [`is_bypass`] let `run()` short-circuit before
//! doing any work.

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Interactive {
    Patch,
    Interactive,
}

/// The result of classifying the git-commit flags the user passed.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ParsedArgs {
    pub(crate) all: bool,
    pub(crate) amend: bool,
    pub(crate) interactive: Option<Interactive>,
    pub(crate) dry_run: bool,
    pub(crate) no_edit: bool,
    pub(crate) no_verify: bool,
    pub(crate) allow_empty: bool,
    /// Steering instructions for the AI (from `-m`/`--message`), empties dropped.
    pub(crate) instructions: Vec<String>,
    /// Path to a template file (from `-t`/`--template`).
    pub(crate) template: Option<String>,
    /// Paths after `--` or bare positional tokens.
    pub(crate) pathspecs: Vec<String>,
    /// Flags forwarded verbatim to `git commit` (`-a`, `--amend`, `-s`, `-S…`, unknowns…).
    pub(crate) passthrough: Vec<String>,
}

impl ParsedArgs {
    /// True when pathspecs scope the commit (git's `--only` mode). Interactive
    /// staging consumes pathspecs itself, so they don't scope the commit there.
    pub(crate) fn scoped(&self) -> bool {
        self.interactive.is_none() && !self.pathspecs.is_empty()
    }

    /// True when the final commit records exactly the index (so an early
    /// pre-commit hook check is meaningful). `-a` and pathspec(`--only`) modes
    /// commit working-tree content that may differ from the index.
    pub(crate) fn commits_index(&self) -> bool {
        !self.all && !self.scoped()
    }
}

/// `true` if the user asked for help (so we print ours instead of spending tokens).
pub(crate) fn wants_help(git_args: &[String]) -> bool {
    git_args.iter().any(|a| a == "-h" || a == "--help")
}

/// `true` if the user supplied a message from another commit/file (`--fixup`,
/// `--squash`, `-C`/`-c`/`-F` and their long forms). There is nothing for the AI
/// to generate, so we hand the original args straight to `git commit`.
pub(crate) fn is_bypass(git_args: &[String]) -> bool {
    git_args.iter().any(|a| is_message_source(a))
}

fn is_message_source(a: &str) -> bool {
    const LONG: [&str; 5] = [
        "--fixup",
        "--squash",
        "--file",
        "--reuse-message",
        "--reedit-message",
    ];
    if LONG
        .iter()
        .any(|f| a == *f || a.strip_prefix(f).is_some_and(|r| r.starts_with('=')))
    {
        return true;
    }
    // Short forms `-F`/`-C`/`-c`, possibly bundled after boolean shorts (`-aC HEAD`).
    if let Some(body) = a.strip_prefix('-')
        && !a.starts_with("--")
        && !body.is_empty()
    {
        return short_token_has_msg_source(body);
    }
    false
}

/// Scan a short-flag bundle for a message-source flag, stopping at the first
/// value-taking flag (whose remainder is its value, not more flags).
fn short_token_has_msg_source(body: &str) -> bool {
    for c in body.chars() {
        match c {
            'C' | 'c' | 'F' => return true,
            'm' | 't' | 'S' | 'u' => return false,
            _ => {}
        }
    }
    false
}

/// Parse the git-commit flags into a [`ParsedArgs`]. Pure (no I/O). Returns `Err`
/// only for malformed *intercepted* flags (e.g. a `-t` with no file); unknown
/// flags are forwarded untouched.
pub(crate) fn classify_args(git_args: &[String]) -> Result<ParsedArgs> {
    let mut p = ParsedArgs::default();
    let mut i = 0;
    while i < git_args.len() {
        let tok = &git_args[i];
        if tok == "--" {
            p.pathspecs.extend(git_args[i + 1..].iter().cloned());
            break;
        } else if let Some(long) = tok.strip_prefix("--") {
            let (name, attached) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (long, None),
            };
            match name {
                "all" => {
                    p.all = true;
                    p.passthrough.push("--all".to_string());
                }
                "amend" => {
                    p.amend = true;
                    p.passthrough.push("--amend".to_string());
                }
                "patch" => p.interactive = Some(Interactive::Patch),
                "interactive" => p.interactive = Some(Interactive::Interactive),
                "dry-run" => p.dry_run = true,
                "edit" => {} // we always add `-e` ourselves
                "no-edit" => {
                    p.no_edit = true;
                    p.passthrough.push("--no-edit".to_string());
                }
                "no-verify" => {
                    p.no_verify = true;
                    p.passthrough.push("--no-verify".to_string());
                }
                "allow-empty" | "allow-empty-message" => {
                    p.allow_empty = true;
                    p.passthrough.push(tok.clone());
                }
                "signoff" => p.passthrough.push("--signoff".to_string()),
                "gpg-sign" => p.passthrough.push(tok.clone()),
                "message" => {
                    let v = take_value(attached, git_args, &mut i, "`--message` requires a value")?;
                    if !v.is_empty() {
                        p.instructions.push(v);
                    }
                }
                "template" => {
                    let v = take_value(attached, git_args, &mut i, "`--template` requires a file")?;
                    if v.is_empty() {
                        return Err(Error::Flags("`--template` requires a file".to_string()));
                    }
                    p.template = Some(v);
                }
                // Known value-taking flags we forward: consume the value too so it
                // isn't mistaken for a pathspec.
                "author" | "date" | "cleanup" | "trailer" | "pathspec-from-file" => {
                    p.passthrough.push(tok.clone());
                    if attached.is_none() {
                        i += 1;
                        match git_args.get(i) {
                            Some(v) => p.passthrough.push(v.clone()),
                            None => {
                                return Err(Error::Flags(format!("`--{name}` requires a value")));
                            }
                        }
                    }
                }
                // Unknown long flag: forward verbatim (don't assume it takes a value).
                _ => p.passthrough.push(tok.clone()),
            }
        } else if tok.starts_with('-') && tok.len() > 1 {
            let body = &tok[1..];
            for (idx, c) in body.char_indices() {
                match c {
                    'a' => {
                        p.all = true;
                        p.passthrough.push("-a".to_string());
                    }
                    's' => p.passthrough.push("-s".to_string()),
                    'e' => {} // we always add `-e` ourselves
                    'n' => {
                        p.no_verify = true;
                        p.passthrough.push("-n".to_string());
                    }
                    'p' => p.interactive = Some(Interactive::Patch),
                    'm' | 't' => {
                        let rest = &body[idx + c.len_utf8()..];
                        let v = if rest.is_empty() {
                            i += 1;
                            git_args
                                .get(i)
                                .ok_or_else(|| Error::Flags(format!("`-{c}` requires a value")))?
                                .clone()
                        } else {
                            rest.to_string()
                        };
                        if c == 'm' {
                            if !v.is_empty() {
                                p.instructions.push(v);
                            }
                        } else if v.is_empty() {
                            return Err(Error::Flags("`-t` requires a file".to_string()));
                        } else {
                            p.template = Some(v);
                        }
                        break;
                    }
                    // `-S` takes an OPTIONAL attached key id and never consumes the
                    // next token (git rejects `-S keyid`); `-u` likewise (`-uno`).
                    'S' | 'u' => {
                        let rest = &body[idx + c.len_utf8()..];
                        p.passthrough.push(format!("-{c}{rest}"));
                        break;
                    }
                    // Unknown short flag: forward as its own boolean flag.
                    other => p.passthrough.push(format!("-{other}")),
                }
            }
        } else {
            p.pathspecs.push(tok.clone());
        }
        i += 1;
    }
    Ok(p)
}

/// Resolve a value-taking flag's argument: the attached `=value`, else the next
/// token (advancing the cursor).
fn take_value(
    attached: Option<&str>,
    git_args: &[String],
    i: &mut usize,
    err: &str,
) -> Result<String> {
    match attached {
        Some(v) => Ok(v.to_string()),
        None => {
            *i += 1;
            git_args
                .get(*i)
                .ok_or_else(|| Error::Flags(err.to_string()))
                .cloned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> ParsedArgs {
        classify_args(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }
    fn parse_err(args: &[&str]) -> String {
        classify_args(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
            .unwrap_err()
            .to_string()
    }
    fn bypass(args: &[&str]) -> bool {
        is_bypass(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }
    /// Vec<String> from &str literals, for ergonomic assert_eq! comparisons.
    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn empty_input() {
        assert_eq!(parse(&[]), ParsedArgs::default());
    }

    #[test]
    fn all_flag() {
        let p = parse(&["-a"]);
        assert!(p.all);
        assert_eq!(p.passthrough, v(&["-a"]));
        let p = parse(&["--all"]);
        assert!(p.all);
        assert_eq!(p.passthrough, v(&["--all"]));
    }

    #[test]
    fn amend_flag() {
        let p = parse(&["--amend"]);
        assert!(p.amend);
        assert_eq!(p.passthrough, v(&["--amend"]));
    }

    #[test]
    fn no_verify_aliases() {
        assert!(parse(&["-n"]).no_verify);
        assert!(parse(&["--no-verify"]).no_verify);
    }

    #[test]
    fn edit_is_deduped() {
        assert!(parse(&["-e"]).passthrough.is_empty());
        assert!(parse(&["--edit"]).passthrough.is_empty());
    }

    #[test]
    fn no_edit_recorded_and_forwarded() {
        let p = parse(&["--no-edit"]);
        assert!(p.no_edit);
        assert_eq!(p.passthrough, v(&["--no-edit"]));
    }

    #[test]
    fn message_forms() {
        assert_eq!(parse(&["-m", "x"]).instructions, v(&["x"]));
        assert_eq!(parse(&["-mx"]).instructions, v(&["x"]));
        assert_eq!(parse(&["--message", "x"]).instructions, v(&["x"]));
        assert_eq!(parse(&["--message=x"]).instructions, v(&["x"]));
        assert!(parse(&["--message="]).instructions.is_empty());
        assert_eq!(parse(&["-m", "a", "-m", "b"]).instructions, v(&["a", "b"]));
        // `-m` is not forwarded to git.
        assert!(parse(&["-m", "x"]).passthrough.is_empty());
    }

    #[test]
    fn template_forms() {
        assert_eq!(parse(&["-t", "f.txt"]).template.as_deref(), Some("f.txt"));
        assert_eq!(parse(&["-tf.txt"]).template.as_deref(), Some("f.txt"));
        assert_eq!(
            parse(&["--template=f.txt"]).template.as_deref(),
            Some("f.txt")
        );
        assert_eq!(
            parse(&["--template", "f.txt"]).template.as_deref(),
            Some("f.txt")
        );
        assert!(parse_err(&["--template"]).contains("template"));
        assert!(parse_err(&["--template="]).contains("template"));
        assert!(parse_err(&["-t"]).contains("requires"));
    }

    #[test]
    fn gpg_sign_optional_value() {
        assert_eq!(parse(&["-S"]).passthrough, v(&["-S"]));
        assert_eq!(parse(&["-Skey"]).passthrough, v(&["-Skey"]));
        assert_eq!(parse(&["--gpg-sign"]).passthrough, v(&["--gpg-sign"]));
        assert_eq!(
            parse(&["--gpg-sign=key"]).passthrough,
            v(&["--gpg-sign=key"])
        );
        // `-S` must NOT consume the next token (git rejects `-S keyid`).
        let p = parse(&["-S", "key"]);
        assert_eq!(p.passthrough, v(&["-S"]));
        assert_eq!(p.pathspecs, v(&["key"]));
    }

    #[test]
    fn untracked_files_optional_value() {
        assert_eq!(parse(&["-u"]).passthrough, v(&["-u"]));
        assert_eq!(parse(&["-uno"]).passthrough, v(&["-uno"]));
    }

    #[test]
    fn signoff_and_dry_run() {
        assert_eq!(parse(&["-s"]).passthrough, v(&["-s"]));
        assert_eq!(parse(&["--signoff"]).passthrough, v(&["--signoff"]));
        assert!(parse(&["--dry-run"]).dry_run);
    }

    #[test]
    fn interactive_flags() {
        assert_eq!(parse(&["-p"]).interactive, Some(Interactive::Patch));
        assert_eq!(parse(&["--patch"]).interactive, Some(Interactive::Patch));
        assert_eq!(
            parse(&["--interactive"]).interactive,
            Some(Interactive::Interactive)
        );
    }

    #[test]
    fn short_bundles() {
        let p = parse(&["-am", "msg"]);
        assert!(p.all);
        assert_eq!(p.instructions, v(&["msg"]));
        assert_eq!(p.passthrough, v(&["-a"]));

        assert_eq!(parse(&["-ammsg"]).instructions, v(&["msg"]));

        let p = parse(&["-aem", "msg"]);
        assert!(p.all);
        assert_eq!(p.instructions, v(&["msg"]));
        assert_eq!(p.passthrough, v(&["-a"])); // `e` dropped

        let p = parse(&["-asn"]);
        assert!(p.all && p.no_verify);
        assert_eq!(p.passthrough, v(&["-a", "-s", "-n"]));

        assert_eq!(parse(&["-asSkey"]).passthrough, v(&["-a", "-s", "-Skey"]));

        let p = parse(&["-sm", "msg"]);
        assert_eq!(p.passthrough, v(&["-s"]));
        assert_eq!(p.instructions, v(&["msg"]));
    }

    #[test]
    fn pathspecs_and_dashdash() {
        assert_eq!(parse(&["file.js"]).pathspecs, v(&["file.js"]));
        assert_eq!(parse(&["a", "b", "c"]).pathspecs, v(&["a", "b", "c"]));
        assert_eq!(
            parse(&["--", "--weird-name"]).pathspecs,
            v(&["--weird-name"])
        );
        let p = parse(&["--"]);
        assert!(p.pathspecs.is_empty());
        assert!(!p.scoped());
    }

    #[test]
    fn forwarded_value_flags_consume_value() {
        let p = parse(&["--author", "Bob", "file.js"]);
        assert_eq!(p.passthrough, v(&["--author", "Bob"]));
        assert_eq!(p.pathspecs, v(&["file.js"]));

        let p = parse(&["--date=2020", "x"]);
        assert_eq!(p.passthrough, v(&["--date=2020"]));
        assert_eq!(p.pathspecs, v(&["x"]));
    }

    #[test]
    fn unknown_flags_forwarded() {
        let p = parse(&["--allow-empty"]);
        assert!(p.allow_empty);
        assert_eq!(p.passthrough, v(&["--allow-empty"]));
        assert_eq!(parse(&["-q"]).passthrough, v(&["-q"]));
        assert_eq!(parse(&["--quiet"]).passthrough, v(&["--quiet"]));
    }

    #[test]
    fn mixed_flags() {
        let p = parse(&["-a", "--amend", "-m", "tweak", "-s", "file.rs"]);
        assert!(p.all && p.amend);
        assert_eq!(p.instructions, v(&["tweak"]));
        assert!(p.passthrough.contains(&"-a".to_string()));
        assert!(p.passthrough.contains(&"--amend".to_string()));
        assert!(p.passthrough.contains(&"-s".to_string()));
        assert_eq!(p.pathspecs, v(&["file.rs"]));
    }

    #[test]
    fn bypass_positives() {
        assert!(bypass(&["--fixup", "HEAD"]));
        assert!(bypass(&["--fixup=HEAD"]));
        assert!(bypass(&["--fixup=amend:HEAD~2"]));
        assert!(bypass(&["--squash"]));
        assert!(bypass(&["--squash=abc"]));
        assert!(bypass(&["-C", "HEAD"]));
        assert!(bypass(&["-CHEAD"]));
        assert!(bypass(&["-c", "HEAD"]));
        assert!(bypass(&["-F", "file"]));
        assert!(bypass(&["--file=x"]));
        assert!(bypass(&["--reuse-message=x"]));
        assert!(bypass(&["--reedit-message"]));
        assert!(bypass(&["-aC", "HEAD"])); // bundled after a boolean short
    }

    #[test]
    fn bypass_negatives() {
        assert!(!bypass(&["-a"]));
        assert!(!bypass(&["-m", "x"]));
        assert!(!bypass(&["--amend"]));
        assert!(!bypass(&["-am", "x"])); // `m` consumes the rest; no C/c/F flag
        assert!(!bypass(&["file.js"]));
        assert!(!bypass(&[]));
    }
}
