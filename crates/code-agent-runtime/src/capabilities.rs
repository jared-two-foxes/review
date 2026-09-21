//! Typed capability markers for code-oriented tool catalogs.
//!
//! ```compile_fail
//! use code_agent_runtime::capabilities::{CodeToolCatalog, ReadOnly};
//! use code_agent_runtime::tools::ReplaceFileContentTool;
//!
//! let mut catalog = CodeToolCatalog::<ReadOnly>::new();
//! let tool: ReplaceFileContentTool = todo!();
//! catalog.register(tool);
//! ```

use agent_kernel::tools::{Tool, ToolCatalog};
use std::marker::PhantomData;

use crate::tools::{
    GetChangeSummaryTool, GetChangedFilesTool, GetProjectGuidanceTool, ListDirectoryTool,
    ReadDiffTool, ReadFileTool, ReplaceFileContentTool, SearchTextTool,
};

pub struct ReadOnly;
pub struct ScopedWrite;

pub trait ToolAllowedIn<C> {}

pub struct CodeToolCatalog<C> {
    inner: ToolCatalog,
    _marker: PhantomData<C>,
}

impl<C> Default for CodeToolCatalog<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C> CodeToolCatalog<C> {
    pub fn new() -> Self {
        Self {
            inner: ToolCatalog::new(),
            _marker: PhantomData,
        }
    }

    pub fn register<T>(&mut self, tool: T)
    where
        T: Tool + ToolAllowedIn<C> + 'static,
    {
        self.inner.register(Box::new(tool));
    }

    pub fn into_inner(self) -> ToolCatalog {
        self.inner
    }
}

impl ToolAllowedIn<ReadOnly> for GetChangeSummaryTool {}
impl ToolAllowedIn<ScopedWrite> for GetChangeSummaryTool {}

impl ToolAllowedIn<ReadOnly> for GetChangedFilesTool {}
impl ToolAllowedIn<ScopedWrite> for GetChangedFilesTool {}

impl ToolAllowedIn<ReadOnly> for ReadDiffTool {}
impl ToolAllowedIn<ScopedWrite> for ReadDiffTool {}

impl ToolAllowedIn<ReadOnly> for ReadFileTool {}
impl ToolAllowedIn<ScopedWrite> for ReadFileTool {}

impl ToolAllowedIn<ReadOnly> for ListDirectoryTool {}
impl ToolAllowedIn<ScopedWrite> for ListDirectoryTool {}

impl ToolAllowedIn<ReadOnly> for SearchTextTool {}
impl ToolAllowedIn<ScopedWrite> for SearchTextTool {}

impl ToolAllowedIn<ReadOnly> for GetProjectGuidanceTool {}
impl ToolAllowedIn<ScopedWrite> for GetProjectGuidanceTool {}

impl ToolAllowedIn<ScopedWrite> for ReplaceFileContentTool {}
