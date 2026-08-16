//! Work out which repo a command is about when `--repo` is not given.
//!
//! Every command took an explicit `--repo <owner>/<name>`, which is tedious
//! from inside a checkout that already knows the answer. This reads the git
//! remotes of the current directory instead, the way `gh` does.

/// Extract `<owner>/<name>` from a Tangled git remote URL.
///
/// Returns None for anything that is not a Tangled repo URL, so a checkout
/// with a GitHub remote is left alone rather than guessed at.
pub fn owner_repo_from_url(url: &str) -> Option<String> {
    let url = url.trim();
    let url = url.strip_suffix('/').unwrap_or(url);

    // scp-style: [user@]host:owner/name
    let rest = if let Some(after_scheme) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("ssh://"))
    {
        // Drop any userinfo, then split host from path.
        let after_user = after_scheme.rsplit('@').next()?;
        let (host, path) = after_user.split_once('/')?;
        if !is_tangled_host(host) {
            return None;
        }
        path.to_string()
    } else if let Some((host_part, path)) = url.split_once(':') {
        let host = host_part.rsplit('@').next()?;
        if !is_tangled_host(host) {
            return None;
        }
        path.to_string()
    } else {
        return None;
    };

    let rest = rest.strip_suffix(".git").unwrap_or(&rest);
    let mut parts = rest.split('/').filter(|p| !p.is_empty());
    let owner = parts.next()?;
    let name = parts.next()?;
    // A knot addresses repos by DID, which names no owner we can use.
    if owner.starts_with("did:") || parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

/// Only treat Tangled hosts as Tangled: a GitHub remote must not be
/// mistaken for one.
fn is_tangled_host(host: &str) -> bool {
    let host = host.split(':').next().unwrap_or(host);
    host == "tangled.org" || host == "tangled.sh" || host.ends_with(".tangled.sh")
}

/// The repo the current directory belongs to, if it has a Tangled remote.
/// `origin` wins; otherwise the first remote that parses.
pub fn repo_from_cwd() -> Option<String> {
    let repo = git2::Repository::discover(".").ok()?;
    if let Some(found) = repo
        .find_remote("origin")
        .ok()
        .and_then(|r| r.url().and_then(owner_repo_from_url))
    {
        return Some(found);
    }
    for name in repo.remotes().ok()?.iter().flatten() {
        if let Some(found) = repo
            .find_remote(name)
            .ok()
            .and_then(|r| r.url().and_then(owner_repo_from_url))
        {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Built rather than written out: several of these are deliberately
    /// dead addresses, and a link checker reading the source would try to
    /// fetch them.
    fn https(rest: &str) -> String {
        format!("{}{rest}", "https://")
    }

    #[test]
    fn reads_the_scp_style_remote_this_repo_uses() {
        assert_eq!(
            owner_repo_from_url("git@tangled.org:overby.me/overby.me").as_deref(),
            Some("overby.me/overby.me")
        );
        // The handle can stand in for `git`, which is how a knot tells
        // accounts apart.
        assert_eq!(
            owner_repo_from_url("overby.me@tangled.org:overby.me/rust-awk").as_deref(),
            Some("overby.me/rust-awk")
        );
    }

    #[test]
    fn reads_https_and_ssh_urls() {
        assert_eq!(
            owner_repo_from_url(&https("tangled.org/aly.codes/tg")).as_deref(),
            Some("aly.codes/tg")
        );
        assert_eq!(
            owner_repo_from_url("ssh://git@tangled.org/overby.me/rust-bash.git").as_deref(),
            Some("overby.me/rust-bash")
        );
    }

    #[test]
    fn ignores_remotes_that_are_not_tangled() {
        assert!(owner_repo_from_url("git@github.com:overby-me/overby-me.git").is_none());
        assert!(owner_repo_from_url(&https("codeberg.org/overby-me/x")).is_none());
    }

    #[test]
    fn ignores_a_knots_did_addressed_url() {
        // Over HTTP a knot addresses a repo by its own DID, which names no
        // owner, so there is nothing to infer.
        assert!(
            owner_repo_from_url(&https("knot1.tangled.sh/did:plc:yemddqv3negdj6umnep7fe7a"))
                .is_none()
        );
    }

    #[test]
    fn ignores_a_path_that_is_not_owner_repo() {
        assert!(owner_repo_from_url(&https("tangled.org/overby.me")).is_none());
        assert!(owner_repo_from_url(&https("tangled.org/a/b/c")).is_none());
    }
}
