#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct FormatVersions {
    pub hook_protocol: u32,
    pub project_registry: u32,
    pub ledger: u32,
    pub normalized_event: u32,
    pub memory: u32,
    pub markdown_projection: u32,
    pub segment_manifest: u32,
    pub service_config: u32,
    pub service_api: u32,
    pub backup_inventory: u32,
}

pub const SUPPORTED_FORMATS: FormatVersions = FormatVersions {
    hook_protocol: 1,
    project_registry: 1,
    ledger: 8,
    normalized_event: 1,
    memory: 1,
    markdown_projection: 1,
    segment_manifest: 1,
    service_config: 2,
    service_api: 1,
    backup_inventory: 1,
};
