use zed_extension_api::{self as zed, Result};

struct MojoExtension;

impl zed::Extension for MojoExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let path = worktree
            .which("mojo-lsp-server")
            .ok_or_else(|| "mojo-lsp-server not found in PATH. Install Mojo or add it to your environment (e.g. via direnv).".to_string())?;

        Ok(zed::Command {
            command: path,
            args: vec![],
            env: Default::default(),
        })
    }
}

zed::register_extension!(MojoExtension);
