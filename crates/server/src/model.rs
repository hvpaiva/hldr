//! The content a request renders from, loaded once per request.

use hldr_core::manifest;
use hldr_core::spec::{PageKindSpec, PageStyle};
use hldr_core::store::{self, Collection, Nav, Page, PageKind, Profile, Site};
use hldr_core::template;
use hldr_core::{Db, Error};
use serde_json::{Value, json};

/// Everything but the pages' markdown, which only the page being rendered
/// needs.
pub struct Model {
    pub site: Site,
    pub profile: Profile,
    pub nav: Nav,
    pub kinds: Vec<PageKind>,
    pub collections: Vec<Collection>,
    /// Every page, drafts included.
    pub pages: Vec<Page>,
}

impl Model {
    pub async fn load(db: &Db) -> Result<Self, Error> {
        Ok(Self {
            site: db.site_config().await?,
            profile: db.profile().await?,
            nav: db.nav().await?.ok_or(Error::Invariant("nav missing"))?,
            kinds: db.page_kinds().await?,
            collections: db.collections().await?,
            pages: db.pages().await?,
        })
    }

    /// The site's name.
    pub fn title(&self) -> &str {
        &self.site.spec.title
    }

    /// A published page under `pages/` that has a route; `index` is `/`.
    pub fn page(&self, name: &str) -> Option<&Page> {
        self.pages.iter().find(|page| {
            page.page.collection.is_none()
                && page.page.name == name
                && !page.page.spec.draft
                && page.page.route().is_some()
        })
    }

    /// The page every 404 shows under its first lines.
    pub fn not_found(&self) -> Option<&Page> {
        self.pages
            .iter()
            .find(|page| page.page.collection.is_none() && page.page.name == "not-found")
    }

    /// The published page whose style is `colorscheme`.
    pub fn picker(&self) -> Option<&Page> {
        self.pages
            .iter()
            .find(|page| page.page.spec.style == PageStyle::Colorscheme && !page.page.spec.draft)
    }

    pub fn collection(&self, name: &str) -> Option<&Collection> {
        self.collections
            .iter()
            .find(|collection| collection.name == name)
    }

    /// A page type by the kind its pages declare.
    pub fn kind(&self, kind: &str) -> Option<&PageKindSpec> {
        self.kinds
            .iter()
            .map(|page_kind| &page_kind.spec)
            .find(|spec| spec.names.kind == kind)
    }

    pub fn kind_of(&self, collection: &Collection) -> Option<&PageKindSpec> {
        self.kind(&collection.spec.kind)
    }

    /// A collection's pages, drafts included, in its order.
    pub fn all_members(&self, collection: &Collection) -> Vec<&Page> {
        let mut members: Vec<&Page> = self
            .pages
            .iter()
            .filter(|page| page.page.collection.as_deref() == Some(collection.name.as_str()))
            .collect();
        store::order(&mut members, collection.spec.order.as_deref());
        members
    }

    /// A collection's published pages, in its order.
    pub fn members(&self, collection: &Collection) -> Vec<&Page> {
        let mut members = self.all_members(collection);
        members.retain(|page| !page.page.spec.draft);
        members
    }

    /// A published page of a collection.
    pub fn member(&self, collection: &Collection, name: &str) -> Option<&Page> {
        self.members(collection)
            .into_iter()
            .find(|page| page.page.name == name)
    }

    /// What templates read: the profile, the site, and the page itself.
    pub fn context(&self, page: Option<&manifest::Page>) -> Value {
        let mut context = json!({
            "profile": self.profile.spec,
            "site": { "title": self.site.spec.title, "values": self.site.spec.values },
        });
        if let Some(page) = page {
            let mut metadata = page.metadata.clone();
            metadata.insert("name".to_owned(), Value::String(page.name.clone()));
            context["page"] = Value::Object(metadata);
        }
        context
    }

    /// A string field with its values filled in.
    pub fn expand(&self, text: &str, page: Option<&manifest::Page>) -> String {
        let context = self.context(page);
        template::expand(text, |path| {
            template::text(template::lookup(&context, path))
        })
    }
}
