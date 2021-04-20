mod documents;
mod formatting;
mod references;
mod rename;
mod semantic_highlighting;
#[cfg(test)]
mod testing;
mod traits;

use crate::config::Config;
use crate::core::codegen::{Analysis, CodegenContext, Definition};
use crate::core::parser::code_map::{LineCol, SpanLoc};
use crate::core::parser::source::ParsingSource;
use crate::core::parser::ParseTree;
use crate::errors::{MosError, MosResult};
use crate::lsp::documents::{
    DidChangeTextDocumentHandler, DidCloseTextDocumentHandler, DidOpenTextDocumentHandler,
};
use crate::lsp::formatting::{FormattingRequestHandler, OnTypeFormattingRequestHandler};
use crate::lsp::references::{
    DocumentHighlightRequestHandler, FindReferencesHandler, GoToDefinitionHandler,
};
use crate::lsp::rename::RenameHandler;
use crate::lsp::semantic_highlighting::SemanticTokensFullRequestHandler;
use crossbeam_channel::{Receiver, Sender};
use lsp_server::{Connection, IoThreads, Message, RequestId};
use lsp_types::notification::Notification;
use lsp_types::{
    DocumentOnTypeFormattingOptions, InitializeParams, OneOf, Position, ServerCapabilities,
    TextDocumentPositionParams, TextDocumentSyncKind, Url,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
pub use traits::*;

impl From<lsp_types::Position> for LineCol {
    fn from(pos: Position) -> Self {
        Self {
            line: pos.line as usize,
            column: pos.character as usize,
        }
    }
}

impl From<LineCol> for lsp_types::Position {
    fn from(lc: LineCol) -> Self {
        Self {
            line: lc.line as u32,
            character: lc.column as u32,
        }
    }
}

impl From<SpanLoc> for lsp_types::Range {
    fn from(s: SpanLoc) -> Self {
        Self {
            start: s.begin.into(),
            end: s.end.into(),
        }
    }
}

impl From<SpanLoc> for lsp_types::Location {
    fn from(sl: SpanLoc) -> Self {
        Self {
            uri: Url::from_file_path(sl.file.name()).unwrap(),
            range: sl.into(),
        }
    }
}

pub struct LspContext {
    connection: Option<(Arc<Connection>, Option<IoThreads>)>,
    tree: Option<Arc<ParseTree>>,
    error: Option<MosError>,
    codegen: Option<CodegenContext>,
    parsing_source: Arc<Mutex<LspParsingSource>>,
    shutdown_senders: Vec<Sender<()>>,
    #[cfg(test)]
    responses: Arc<Mutex<Vec<lsp_server::Response>>>,
}

pub struct LspParsingSource {
    files: HashMap<PathBuf, String>,
}

impl LspParsingSource {
    fn new() -> Self {
        Self {
            files: HashMap::new(),
        }
    }

    pub fn insert(&mut self, path: &Path, src: &str) {
        log::trace!("Inserted file: {:?}", &path);
        self.files.insert(path.to_path_buf(), src.to_string());
    }

    pub fn remove(&mut self, path: &Path) {
        self.files.remove(path);
    }
}

impl ParsingSource for LspParsingSource {
    fn get_contents(&self, path: &Path) -> MosResult<String> {
        log::trace!("Trying to get file: {:?}", &path);
        let path = path.to_path_buf();
        if let Some(file) = self.files.get(&path) {
            Ok(file.clone())
        } else {
            let data = fs_err::read_to_string(path)?;
            Ok(data)
        }
    }
}

impl LspContext {
    pub(crate) fn new() -> Self {
        Self {
            connection: None,
            tree: None,
            error: None,
            codegen: None,
            parsing_source: Arc::new(Mutex::new(LspParsingSource::new())),
            shutdown_senders: vec![],
            #[cfg(test)]
            responses: Arc::new(Mutex::new(vec![])),
        }
    }

    pub(crate) fn listen_stdio(&mut self) {
        let (connection, io_threads) = Connection::stdio();
        self.connection = Some((Arc::new(connection), Some(io_threads)));
    }

    #[cfg(test)]
    pub(crate) fn listen_memory(&mut self) -> Connection {
        let (connection, test_connection) = Connection::memory();
        self.connection = Some((Arc::new(connection), None));
        test_connection
    }

    pub fn add_shutdown_handler(&mut self) -> Receiver<()> {
        let (s, r) = crossbeam_channel::bounded(0);
        self.shutdown_senders.push(s);
        r
    }

    pub fn config(&self) -> Option<Config> {
        self.parsing_source
            .lock()
            .unwrap()
            .try_get_contents(&Path::new("mos.toml"))
            .map(|toml| Config::from_toml(&toml).ok())
            .flatten()
    }

    #[cfg(test)]
    fn working_directory(&self) -> PathBuf {
        use crate::lsp::testing::test_root;
        test_root()
    }

    #[cfg(not(test))]
    fn working_directory(&self) -> PathBuf {
        use path_absolutize::Absolutize;
        PathBuf::from(".").absolutize().unwrap().into()
    }

    pub fn codegen(&self) -> Option<&CodegenContext> {
        self.codegen.as_ref()
    }

    pub fn analysis(&self) -> Option<&Analysis> {
        self.codegen.as_ref().map(|c| c.analysis())
    }

    fn connection(&self) -> Option<Arc<Connection>> {
        self.connection.as_ref().map(|c| c.0.clone())
    }

    fn parsing_source(&self) -> MutexGuard<LspParsingSource> {
        self.parsing_source.lock().unwrap()
    }

    #[cfg(test)]
    fn responses(&self) -> Vec<lsp_server::Response> {
        self.responses.lock().unwrap().clone()
    }

    fn publish_notification<N: Notification>(&self, params: N::Params) -> MosResult<()> {
        let params = serde_json::to_value(&params).unwrap();
        let n = lsp_server::Notification {
            method: N::METHOD.into(),
            params,
        };
        if let Some(conn) = self.connection() {
            conn.sender.send(Message::Notification(n))?;
        }
        Ok(())
    }

    fn send_response<R: DeserializeOwned + Serialize>(
        &self,
        id: RequestId,
        result: R,
    ) -> MosResult<()> {
        let result = serde_json::to_value(&result).unwrap();
        let response = lsp_server::Response {
            id,
            result: Some(result),
            error: None,
        };

        #[cfg(test)]
        {
            self.responses.lock().unwrap().push(response.clone());
        }
        if let Some(conn) = self.connection() {
            conn.sender.send(Message::Response(response))?;
        }
        Ok(())
    }

    fn join(self) -> MosResult<()> {
        if let Some(io) = self.connection.unwrap().1 {
            io.join()?;
        }
        Ok(())
    }

    fn find_definitions<'a>(&'a self, pos: &'a TextDocumentPositionParams) -> Vec<&'a Definition> {
        if let Some(analysis) = self.analysis() {
            return analysis.find(pos.text_document.uri.to_file_path().unwrap(), pos.position);
        }

        vec![]
    }
}

pub struct LspServer {
    context: Arc<Mutex<LspContext>>,
    request_handlers: HashMap<&'static str, Box<dyn UntypedRequestHandler>>,
    notification_handlers: HashMap<&'static str, Box<dyn UntypedNotificationHandler>>,
}

impl LspServer {
    pub fn new(context: LspContext) -> Self {
        let request_handlers = HashMap::new();
        let notification_handlers = HashMap::new();

        let mut lsp = Self {
            context: Arc::new(Mutex::new(context)),
            request_handlers,
            notification_handlers,
        };

        lsp.register_request_handler(SemanticTokensFullRequestHandler {});
        lsp.register_request_handler(FormattingRequestHandler {});
        lsp.register_request_handler(OnTypeFormattingRequestHandler {});
        lsp.register_request_handler(GoToDefinitionHandler {});
        lsp.register_request_handler(FindReferencesHandler {});
        lsp.register_request_handler(DocumentHighlightRequestHandler {});
        lsp.register_request_handler(RenameHandler {});
        lsp.register_notification_handler(DidOpenTextDocumentHandler {});
        lsp.register_notification_handler(DidChangeTextDocumentHandler {});
        lsp.register_notification_handler(DidCloseTextDocumentHandler {});

        lsp
    }

    pub fn context(&self) -> Arc<Mutex<LspContext>> {
        self.context.clone()
    }

    pub fn lock_context(&self) -> MutexGuard<LspContext> {
        self.context.lock().unwrap()
    }

    pub fn start(mut self) -> MosResult<()> {
        log::info!("Starting MOS language server");

        let caps = ServerCapabilities {
            semantic_tokens_provider: Some(semantic_highlighting::caps().into()),
            text_document_sync: Some(TextDocumentSyncKind::Full.into()),
            references_provider: Some(OneOf::Left(true)),
            document_formatting_provider: Some(OneOf::Left(true)),
            document_on_type_formatting_provider: Some(DocumentOnTypeFormattingOptions {
                first_trigger_character: "}".to_string(),
                more_trigger_character: None,
            }),
            document_highlight_provider: Some(OneOf::Left(true)),
            rename_provider: Some(OneOf::Left(true)),
            definition_provider: Some(OneOf::Left(true)),
            ..Default::default()
        };
        let server_capabilities = serde_json::to_value(&caps).unwrap();
        let initialization_params = self
            .lock_context()
            .connection()
            .unwrap()
            .initialize(server_capabilities)?;
        self.main_loop(initialization_params)?;
        Arc::try_unwrap(self.context)
            .ok()
            .unwrap()
            .into_inner()
            .unwrap()
            .join()?;

        log::info!("Shutting down MOS language server");
        Ok(())
    }

    pub fn handle_message(&mut self, msg: Message) -> MosResult<()> {
        log::trace!("Handling message: {:?}", msg);

        let cloned_ctx = self.context.clone();
        let mut ctx = cloned_ctx.lock().unwrap();

        match msg {
            Message::Request(req) => match self.request_handlers.get(req.method.as_str()) {
                Some(handler) => {
                    handler.handle(&mut ctx, req)?;
                }
                None => {
                    if req.method == "shutdown" {
                        for sender in &ctx.shutdown_senders {
                            sender.send(())?;
                        }
                        if ctx.connection().unwrap().handle_shutdown(&req)? {
                            return Ok(());
                        }
                    } else {
                        log::trace!("unknown request: {:?}", req);
                    }
                }
            },
            Message::Response(_resp) => {}
            Message::Notification(not) => {
                match self.notification_handlers.get(not.method.as_str()) {
                    Some(handler) => {
                        handler.handle(&mut ctx, not)?;
                    }
                    None => {
                        log::trace!("unknown notification: {:?}", not);
                    }
                }
            }
        }

        Ok(())
    }

    fn main_loop(&mut self, params: serde_json::Value) -> MosResult<()> {
        let connection = self.lock_context().connection().unwrap();

        let _params: InitializeParams = serde_json::from_value(params).unwrap();
        for msg in &connection.receiver {
            self.handle_message(msg)?;
        }

        Ok(())
    }

    fn register_request_handler<T: 'static + UntypedRequestHandler>(&mut self, handler: T) {
        self.request_handlers
            .insert(handler.method(), Box::new(handler));
    }

    fn register_notification_handler<T: 'static + UntypedNotificationHandler>(
        &mut self,
        handler: T,
    ) {
        self.notification_handlers
            .insert(handler.method(), Box::new(handler));
    }
}
