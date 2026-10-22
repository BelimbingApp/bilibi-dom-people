//! The reference data pages (Bilimbi `Bilimbi.People.ReferenceData.Web.IndexLive`):
//! `/people/references`, the authorized company chooser, and
//! `/people/companies/{company_id}/references`, one company's entries,
//! aliases and calendar exceptions. Both routes are one live page over the
//! [`ReferenceData`] facade. The page re-reads the actor's capability and
//! company reach before every event, checks it again before a write-shaped
//! event, and the facade checks it once more inside the write. A refusal or a
//! lost grant is a flash, never a page-clearing error.
mod references;

use crate::ReferenceData;
use bilibi_database::{Db, Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveState, MountFuture, Page, PageBody, PageFuture, PageRequest},
    FlashMap, Html, StatusKind, NO_FLASH,
};
use std::sync::Arc;

pub(crate) struct Core {
    db: Db,
    references: ReferenceData,
}

pub(crate) struct Pages(Arc<Core>);
impl Pages {
    pub(crate) fn new(db: Db, references: ReferenceData) -> Self {
        Self(Arc::new(Core { db, references }))
    }
}

impl Page<Scope, Error> for Pages {
    fn title(&self, operation: &str) -> Option<&'static str> {
        match operation {
            "references" => Some("People references"),
            _ => None,
        }
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let scope = request.scope.ok_or(Error::Unauthorized)?;
            let html = match request.operation {
                "references" => {
                    let mut page = references::References::new(self.0.clone(), request.path)?;
                    page.load(scope).await?;
                    page.render()?
                }
                _ => return Err(Error::NotFound),
            };
            Ok(PageBody {
                html,
                live: true,
                title: None,
            })
        })
    }
    fn mount<'a>(&'a self, request: PageRequest<'a, Scope>) -> MountFuture<'a, Scope, Error> {
        Box::pin(async move {
            let state: Box<dyn LiveState<Scope, Error>> = match request.operation {
                "references" => Box::new(references::References::new(
                    self.0.clone(),
                    request.path,
                )?),
                _ => return Ok(None),
            };
            Ok(Some(state))
        })
    }
}

/// The one message a page shows, and the flash-group dismissal that clears it.
#[derive(Default)]
struct Flash(Option<(StatusKind, String)>);
impl Flash {
    fn success(&mut self, message: impl Into<String>) {
        self.0 = Some((StatusKind::Success, message.into()));
    }
    fn error(&mut self, message: impl Into<String>) {
        self.0 = Some((StatusKind::Error, message.into()));
    }
    fn clear(&mut self, key: Option<&str>) {
        if self
            .0
            .as_ref()
            .is_some_and(|(kind, _)| Some(kind.name()) == key)
        {
            self.0 = None;
        }
    }
    fn map(&self) -> FlashMap<'_> {
        let mut flash = NO_FLASH;
        if let Some((kind, message)) = &self.0 {
            match kind {
                StatusKind::Success => flash.success = message,
                StatusKind::Error => flash.error = message,
                StatusKind::Warning => flash.warning = message,
                StatusKind::Info => flash.info = message,
            }
        }
        flash
    }
}

fn render(fragment: &impl askama::Template) -> Result<Html> {
    Html::render(fragment).map_err(|_| Error::Internal)
}
