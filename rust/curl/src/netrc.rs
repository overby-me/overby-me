//! Minimal `.netrc` parser used by `--netrc` / `--netrc-optional`.
//!
//! Format (whitespace-separated tokens):
//!   `machine HOST login USER password PASS`
//!   `default login USER password PASS`
//! Quoted values support `\r`, `\n`, `\t`, `\\`, `\"` escapes.
//! For multiple `machine HOST` blocks the LAST one wins (matches curl, see
//! test 478). `default` is used only if no `machine` block matched.

use std::path::Path;

/// Look up `host` in `path`, optionally constrained to a specific `login`.
/// Returns `(login, password)` from the first complete match. Either field
/// may be `None` if the file omits it. Mirrors curl's `lib/netrc.c` state
/// machine: scan top-to-bottom; once we've seen a `machine HOST` block that
/// matches `host` (and a `login LOGIN` line matching `specific_login` when
/// one is given), the next `password PASS` finishes the search.
pub(crate) type LoginPassword = (Option<String>, Option<String>);

pub(crate) fn lookup(
    path: &Path,
    host: &str,
    specific_login: Option<&str>,
) -> Result<Option<LoginPassword>, String> {
    let contents = std::fs::read_to_string(path).map_err(|e| format!("read netrc: {e}"))?;
    let host_lc = host.to_ascii_lowercase();

    #[derive(PartialEq)]
    enum State {
        Nothing,
        HostFound,
        HostValid,
    }

    let tokens = tokenize(&contents)?;
    let mut iter = tokens.into_iter();
    let mut state = State::Nothing;
    let mut login: Option<String> = None;
    let mut password: Option<String> = None;
    let mut our_login = specific_login.is_none();
    let mut found_password = false;
    // Snapshot of (login, password) the first time both arrived in a
    // matching machine block — used as a fallback when we then walk into
    // a later machine block without ever locking in `our_login`.
    let mut default_pair: Option<(Option<String>, Option<String>)> = None;

    while let Some(tok) = iter.next() {
        match state {
            State::Nothing => {
                if tok == "machine" {
                    state = State::HostFound;
                    found_password = false;
                    password = None;
                    if specific_login.is_none() {
                        login = None;
                    }
                    our_login = specific_login.is_none();
                } else if tok == "default" {
                    state = State::HostValid;
                }
            }
            State::HostFound => {
                let host_match = tok.eq_ignore_ascii_case(&host_lc);
                state = if host_match {
                    State::HostValid
                } else {
                    State::Nothing
                };
            }
            State::HostValid => {
                match tok.as_str() {
                    "login" => {
                        let val = iter.next().unwrap_or_default();
                        if let Some(want) = specific_login {
                            our_login = val == want;
                        } else {
                            our_login = true;
                            login = Some(val);
                        }
                    }
                    "password" | "passwd" => {
                        let val = iter.next().unwrap_or_default();
                        password = Some(val);
                        if our_login {
                            found_password = true;
                        }
                    }
                    "account" => {
                        let _ = iter.next();
                    }
                    "machine" => {
                        if found_password {
                            break;
                        }
                        state = State::HostFound;
                        found_password = false;
                        password = None;
                        if specific_login.is_none() {
                            login = None;
                        }
                        our_login = specific_login.is_none();
                    }
                    "default" => {
                        if found_password {
                            break;
                        }
                        // stay in HostValid for default block
                        found_password = false;
                        password = None;
                        if specific_login.is_none() {
                            login = None;
                        }
                        our_login = specific_login.is_none();
                    }
                    _ => {}
                }
                if found_password && default_pair.is_none() {
                    default_pair = Some((login.clone(), password.clone()));
                }
            }
        }
    }

    // Curl's cleanup path returns whatever final-state fields are set even
    // if `done` (FOUND_LOGIN | FOUND_PASSWORD with our_login) was never
    // reached. Accept these end-of-file states:
    //   * our_login is true (login from netrc matched specific_login, or
    //     no specific_login was given) and login or password is set.
    //   * a password was seen in a matching block that had NO login at
    //     all — curl falls back to the URL-supplied login (test 685).
    let no_login_in_block = login.is_none();
    let accept = found_password
        || (our_login && (login.is_some() || password.is_some()))
        || (specific_login.is_some() && password.is_some() && no_login_in_block);
    if accept {
        return Ok(Some((
            specific_login.map(|s| s.to_string()).or(login),
            password,
        )));
    }
    if let Some((l, p)) = default_pair {
        return Ok(Some((specific_login.map(|s| s.to_string()).or(l), p)));
    }
    Ok(None)
}

/// Split `.netrc` text into tokens. Tokens are whitespace-separated, but a
/// `"…"` quoted run becomes a single token with `\` escapes processed.
/// An unterminated quoted run returns an error (test 680).
fn tokenize(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_whitespace() {
            chars.next();
            continue;
        }
        if c == '#' {
            // Skip to end of line.
            for c in chars.by_ref() {
                if c == '\n' {
                    break;
                }
            }
            continue;
        }
        if c == '"' {
            chars.next();
            let mut tok = String::new();
            let mut closed = false;
            #[allow(clippy::while_let_on_iterator)]
            while let Some(c) = chars.next() {
                match c {
                    '"' => {
                        closed = true;
                        break;
                    }
                    '\\' => {
                        if let Some(esc) = chars.next() {
                            tok.push(match esc {
                                'r' => '\r',
                                'n' => '\n',
                                't' => '\t',
                                other => other,
                            });
                        }
                    }
                    other => tok.push(other),
                }
            }
            if !closed {
                return Err("unterminated quote in netrc".to_string());
            }
            out.push(tok);
        } else {
            let mut tok = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_whitespace() {
                    break;
                }
                tok.push(c);
                chars.next();
            }
            out.push(tok);
        }
    }
    Ok(out)
}
