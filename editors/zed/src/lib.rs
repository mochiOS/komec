use zed_extension_api::{self as zed, settings::LspSettings};

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
        let settings = LspSettings::for_worktree(language_server_id.as_ref(), worktree)?;

        let configured_path = settings
            .binary
            .as_ref()
            .and_then(|binary| binary.path.clone());

        let configured_args = settings
            .binary
            .as_ref()
            .and_then(|binary| binary.arguments.clone())
            .unwrap_or_default();

        let (command, args) = if let Some(command) = configured_path {
            (command, configured_args)
        } else if let Some(command) = worktree.which("kome-lsp") {
            (command, configured_args)
        } else if worktree.read_text_file("crates/lsp/Cargo.toml").is_ok() {
            let cargo = worktree.which("cargo").ok_or_else(missing_lsp_message)?;
            let manifest = format!("{}/Cargo.toml", worktree.root_path());
            let mut args = vec![
                "run".to_string(),
                "--quiet".to_string(),
                "-p".to_string(),
                "kome_lsp".to_string(),
                "--bin".to_string(),
                "kome-lsp".to_string(),
                "--manifest-path".to_string(),
                manifest,
                "--".to_string(),
            ];
            args.extend(configured_args);
            (cargo, args)
        } else {
            let home = worktree
                .shell_env()
                .into_iter()
                .find_map(|(name, value)| (name == "HOME").then_some(value))
                .ok_or_else(missing_lsp_message)?;
            (format!("{home}/.kome/bin/kome-lsp"), configured_args)
        };

        let env = settings
            .binary
            .and_then(|binary| binary.env)
            .unwrap_or_default()
            .into_iter()
            .collect();

        Ok(zed::Command { command, args, env })
    }

    fn language_server_initialization_options(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<Option<zed::serde_json::Value>> {
        let settings = LspSettings::for_worktree(language_server_id.as_ref(), worktree)?;

        Ok(settings.initialization_options)
    }

    fn language_server_workspace_configuration(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<Option<zed::serde_json::Value>> {
        let settings = LspSettings::for_worktree(language_server_id.as_ref(), worktree)?;

        Ok(settings.settings)
    }
}

fn missing_lsp_message() -> String {
    concat!(
        "Could not find the Kome Language Server. ",
        "Install kome-lsp in PATH or ~/.kome/bin, or configure ",
        "lsp.kome-lsp.binary.path in Zed settings."
    )
    .to_string()
}

zed::register_extension!(KomeExtension);
