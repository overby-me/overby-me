# Vendored skills

Copied verbatim from [schirrmacher/coding](https://github.com/schirrmacher/coding),
`skills/`, at revision `f1b29f5b44b29416e961cf3eacfe4342d3cbd5a6`.

Vendored rather than taken as a flake input because they are thirteen markdown
files with no build and no dependencies: an input would pin a whole repository,
of which this is one directory, and would put a fetch between the shell and a
file you want to read while editing it.

Upstream carries no licence file. Refreshing them is a re-copy from that path
at a newer revision. The only local edit is quoting the `description` of
`deepcomment` and `prompt`: upstream leaves a colon followed by a space unquoted, which is
invalid YAML and makes Zed reject the skill. Re-apply it after a refresh.
