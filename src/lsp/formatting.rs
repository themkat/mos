use crate::errors::MosResult;
use crate::formatting::{format, FormattingOptions};
use crate::impl_request_handler;
use crate::lsp::{LspContext, RequestHandler};
use lsp_types::{DocumentFormattingParams, TextEdit};

pub struct FormattingRequestHandler {}

impl_request_handler!(FormattingRequestHandler);

impl RequestHandler<lsp_types::request::Formatting> for FormattingRequestHandler {
    fn handle(
        &self,
        ctx: &mut LspContext,
        _params: DocumentFormattingParams,
    ) -> MosResult<Option<Vec<TextEdit>>> {
        match &ctx.analysis {
            Some(analysis) => {
                let new_text = format(analysis.tree.clone(), FormattingOptions::default());
                let edit = TextEdit {
                    range: lsp_types::Range {
                        start: lsp_types::Position {
                            line: 0,
                            character: 0,
                        },
                        end: lsp_types::Position {
                            line: u32::MAX - 1,
                            character: 0,
                        },
                    },
                    new_text,
                };
                Ok(Some(vec![edit]))
            }
            _ => Ok(None),
        }
    }
}