use zed_extension_api::{
    self as zed,
    settings::LspSettings,
};

struct KomeExtension;

impl zed::Extension for KomeExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        let settings = LspSettings::for_worktree(
            language_server_id.as_ref(),
            worktree,
        )?;

        let configured_path = settings
            .binary
            .as_ref()
            .and_then(|binary| binary.path.clone());

        let command = configured_path
            .or_else(|| worktree.which("kome-lsp"))
            .ok_or_else(|| {
                concat!(
                    "Could not find the Kome Language Server. ",
                    "Install kome-lsp in PATH or configure ",
                    "lsp.kome-lsp.binary.path in Zed settings."
                )
                    .to_string()
            })?;

        let args = settings
            .binary
            .as_ref()
            .and_then(|binary| binary.arguments.clone())
            .unwrap_or_default();

        let env = settings
            .binary
            .and_then(|binary| binary.env)
            .unwrap_or_default()
            .into_iter()
            .collect();

        Ok(zed::Command {
            command,
            args,
            env,
        })
    }

    fn language_server_initialization_options(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<Option<zed::serde_json::Value>> {
        let settings = LspSettings::for_worktree(
            language_server_id.as_ref(),
            worktree,
        )?;

        Ok(settings.initialization_options)
    }

    fn language_server_workspace_configuration(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<Option<zed::serde_json::Value>> {
        let settings = LspSettings::for_worktree(
            language_server_id.as_ref(),
            worktree,
        )?;

        Ok(settings.settings)
    }
}

zed::register_extension!(KomeExtension);