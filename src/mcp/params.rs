//! Typed MCP inputs. Mutations require an explicit preview/apply choice.

use rmcp::schemars;
use serde::Deserialize;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct List {
    pub query: Option<String>,
    #[serde(default)]
    pub exact: bool,
    #[serde(default)]
    pub bare: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Inspection {
    /// Maximum concurrent repositories, from 1 through 32. Defaults to 4.
    pub jobs: Option<u8>,
    /// Timeout for each network subprocess, from 1 through 3600 seconds.
    pub timeout: Option<u64>,
    /// Inspect cached remote refs. Defaults to true; false fetches and updates refs.
    pub no_fetch: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sync {
    /// True previews using cached refs; false applies safe updates to all configured checkouts.
    pub dry_run: bool,
    pub jobs: Option<u8>,
    pub timeout: Option<u64>,
    /// On apply, use cached refs instead of fetching. Defaults to false.
    #[serde(default)]
    pub no_fetch: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Find {
    /// Search directory. Required to keep the search scope explicit.
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Adopt {
    pub path: String,
    /// Keep the checkout in place and register it in the manifest.
    #[serde(default)]
    pub register: bool,
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Migrate {
    pub path: String,
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct New {
    pub name: String,
    pub namespace: Option<String>,
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub repository: String,
    #[serde(default)]
    pub bare: bool,
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Restore {
    pub dry_run: bool,
    pub timeout: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Archive {
    /// Exact checkout path or an unambiguous repository query.
    pub query: String,
    #[serde(default)]
    pub bare: bool,
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrashRestore {
    /// Archive identifier returned by archive or `trash_list`.
    pub id: String,
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Forge {
    Github,
    Gitlab,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    Private,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Publish {
    /// Checkout to publish. This tool never changes the server's working directory.
    pub path: String,
    /// Explicit owner/name, or group/subgroup/name on GitLab.
    pub repository: String,
    pub visibility: Visibility,
    pub forge: Option<Forge>,
    pub host: Option<String>,
    #[serde(default)]
    pub resume: bool,
    pub dry_run: bool,
    pub timeout: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Partial {
    Blobless,
    Treeless,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "typed equivalents of independent clone flags"
)]
pub struct Get {
    /// One or more Git repository URLs or host/owner/name specifications.
    pub repositories: Vec<String>,
    pub dry_run: bool,
    #[serde(default)]
    pub update: bool,
    #[serde(default)]
    pub ssh: bool,
    #[serde(default)]
    pub shallow: bool,
    #[serde(default)]
    pub no_recursive: bool,
    #[serde(default)]
    pub bare: bool,
    pub branch: Option<String>,
    pub partial: Option<Partial>,
    pub jobs: Option<u8>,
    pub timeout: Option<u64>,
}
