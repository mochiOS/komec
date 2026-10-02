use kome_lsp::definition::definition_at;
use kome_lsp::diagnostics::syntax_diagnostics;
use std::collections::HashMap;
use tokio::sync::RwLock;
use tower_lsp::lsp_types::{
    CompletionOptions, CompletionParams, CompletionResponse, GotoDefinitionParams,
    GotoDefinitionResponse, OneOf,
};
use tower_lsp::{
    Client, LanguageServer, LspService, Server,
    jsonrpc::Result,
    lsp_types::{
        DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
        InitializeParams, InitializeResult, InitializedParams, MessageType, PositionEncodingKind,
        ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
    },
};

#[derive(Debug)]
struct Backend {
    client: Client,
    documents: RwLock<HashMap<Url, String>>,
}

impl Backend {
    async fn publish_syntax_diagnostics(&self, uri: Url, source: &str, version: Option<i32>) {
        let diagnostics = syntax_diagnostics(source);

        self.client
            .publish_diagnostics(uri, diagnostics, version)
            .await;
    }

    async fn clear_diagnostics(&self, uri: Url) {
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                position_encoding: Some(PositionEncodingKind::UTF16),

                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),

                definition_provider: Some(OneOf::Left(true)),

                completion_provider: Some(CompletionOptions::default()),

                ..ServerCapabilities::default()
            },

            server_info: Some(ServerInfo {
                name: "kome-lsp".to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Kome language server initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let document = params.text_document;

        self.documents
            .write()
            .await
            .insert(document.uri.clone(), document.text.clone());

        self.publish_syntax_diagnostics(document.uri, &document.text, Some(document.version))
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let Some(change) = params.content_changes.into_iter().last() else {
            return;
        };

        let uri = params.text_document.uri;

        self.documents
            .write()
            .await
            .insert(uri.clone(), change.text.clone());

        self.publish_syntax_diagnostics(uri, &change.text, Some(params.text_document.version))
            .await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;

        self.documents.write().await.remove(&uri);

        self.clear_diagnostics(uri).await;
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;

        let position = params.text_document_position_params.position;

        self.client
            .log_message(
                MessageType::INFO,
                format!(
                    "definition request: {}:{}:{}",
                    uri, position.line, position.character,
                ),
            )
            .await;

        let documents = self.documents.read().await;

        let Some(source) = documents.get(&uri) else {
            self.client
                .log_message(
                    MessageType::ERROR,
                    format!("definition failed: document is not cached: {uri}",),
                )
                .await;

            return Ok(None);
        };

        let definition = definition_at(&uri, source, position);

        self.client
            .log_message(
                MessageType::INFO,
                format!(
                    "definition result: {}",
                    if definition.is_some() {
                        "found"
                    } else {
                        "not found"
                    },
                ),
            )
            .await;

        Ok(definition.map(GotoDefinitionResponse::Scalar))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let documents = self.documents.read().await;
        let Some(source) = documents.get(&uri) else {
            return Ok(None);
        };
        Ok(Some(CompletionResponse::Array(completion_at(
            source, position,
        ))))
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        documents: RwLock::new(HashMap::new()),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
use kome_lsp::completion::completion_at;
