$env.config = {
  edit_mode: vi
  show_banner: false
  keybindings: [{
    name: path_picker
    modifier: control
    keycode: char_y
    mode: [emacs vi_normal vi_insert]
    event: {
      send: executehostcommand
      cmd: "let f = (mktemp); zellij action dump-screen $f; open $f | uf; rm $f"
    }
  }]
}

def zellij-update-tabname [] {
    if ("ZELLIJ" in $env) {
        let pwd_path = pwd | str replace $env.HOME "~";
        let session_path = $env.ZELLIJ_SESSION_NAME | str replace --all "|" "/";
        mut $tabname = $"($pwd_path | str replace $session_path ".") ❯";

        try {
            let cmd = (commandline | into string | str substring 0..15);
            if ($cmd == "ssh") {
                let ssh = (commandline | into string | split row " " | get 1);
                $tabname = $"($ssh) ❯";
            } else if ($pwd_path | str starts-with $session_path) {
              $tabname = $"($pwd_path | str replace $session_path ".") ❯ ($cmd)";
            } else {
              $tabname = $"($pwd_path) ❯ ($cmd)";
            }
        };

        zellij action rename-tab $tabname;
    }
}

$env.config.hooks = {
    pre_execution: [
        { zellij-update-tabname }
    ],
    env_change: {
        PWD: [
            { zellij-update-tabname }
        ]
    }
    # Fix for: https://github.com/nushell/nushell/issues/11950
    display_output: {||
        if (term size).columns >= 100 {
          table -e
        } else {
          table
        }
        | if (($in | describe) =~ "^string(| .*)") and ($in | str contains (ansi cursor_position)) {
          str replace --no-expand --all (ansi cursor_position) ""
        } else {
          print -n --raw $in
        }
    }
}

$env.PATH = ($env.PATH | split row (char esep))

def --env uo [] { let res = uf | $in; cd $res }

def jhash [] {jj log -r @ --no-graph -T 'commit_id' | tr -d '\\n' | wl-copy; jj log -r @ --no-graph -T 'commit_id'}

def show [] {to json | jless}

def jjj [] {
  jj git push
  gh pr create --fill
  gh pr comment --body 'bors merge'
}

def dhost [num: int] {
  let known_hosts = open ~/.ssh/known_hosts | lines;
  $known_hosts | enumerate | where index != ($num - 1) | get item | save -f ~/.ssh/known_hosts
}

def jdf [branch: string] {
  jco $branch
  let elems = $branch | split row "/"
  if ($elems | length) == 4 {
    jco $"bump-($elems | last 2 | str join "-")"

  } else if ($elems | length) == 3 {
    jco $"bump-($elems | get 2)"
  }
  jj squash
  jj git push
  gh pr create --fill
}

def bin64 [] {
  xxd -r -p | base64 -w 0
}

def unbin64 [] {
  base64 -d | xxd -p -c 0
}

def --env assume [profile?: string = ""] {
  let granted_output = assumego $profile
  let granted = $granted_output | lines | get 0 | split row " "
  load-env {
    AWS_ACCESS_KEY_ID: $granted.1,
    AWS_SECRET_ACCESS_KEY: $granted.2,
    AWS_SESSION_TOKEN: $granted.3,
    AWS_PROFILE: $granted.4,
    AWS_REGION: $granted.5,
    AWS_DEFAULT_REGION: $granted.5,
    AWS_SESSION_EXPIRATION: $granted.6,
    AWS_CREDENTIAL_EXPIRATION: $granted.6,
    GRANTED_SSO: $granted.7,
    GRANTED_SSO_START_URL: $granted.8,
    GRANTED_SSO_ROLE_NAME: $granted.9,
    GRANTED_SSO_REGION: $granted.10,
    GRANTED_SSO_ACCOUNT_ID: $granted.11,
  }
 }

def yarn-lock-update [] {
  try { jrm }
  let root = jj workspace root
  jj restore $"($root)/.pnp.cjs" $"($root)/yarn.lock"
  yarn
  jj file track $"($root)/.pnp.cjs" $"($root)/yarn.lock"
}

def jco [branch_name: string] {
    jj git fetch

    let bookmark_exists = (jj bookmark list | lines | any {|line| $line | str contains $branch_name})
    let remote_exists = (jj bookmark list --all | lines | any {|line| $line | str contains $"($branch_name)@origin"})

    if $bookmark_exists {
        jj new $branch_name
    } else if $remote_exists {
        jj new $"($branch_name)@origin"
        jj bookmark create $branch_name
    } else {
        jj bookmark create $branch_name
    }
}

def jcom [] {
  jj git fetch
  let default_branch = (jj config get git.default-remote-bookmark? | default "main")
  jj new $"($default_branch)@origin"
}

def jrm [] {
  jj git fetch
  let default_branch = (jj config get git.default-remote-bookmark? | default "main")
  jj rebase -d $"($default_branch)@origin"
}

def jreset [] {
  jj git fetch
  let default_branch = (jj config get git.default-remote-bookmark? | default "main")
  jj squash --into (jj log -r $"roots(\@::($default_branch)\@origin)" --no-graph -T 'change_id' | lines | first)
}

# Use Zellij-cwd in Zed terminal
if ($env.ZED_TERM?  == "true") and ($env.ZELLIJ? == null) {
    zellij-cwd
}
