use std::path::Path;

use anyhow::Result;
use brain_domain::ProjectId;
use brain_service::ServiceLaunchConfig;
use brain_store::{
    BasicMemoryCli, BasicMemoryIndexer, BasicMemoryReport, EventLedger, MarkdownProjector,
    ProcessBasicMemoryCli, ProjectionReport, ProjectionVerification,
};

pub fn rebuild_markdown(brain_home: &Path, project_id: ProjectId) -> Result<ProjectionReport> {
    let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(brain_home))?;
    let project = config.project(Some(project_id))?;
    let ledger = EventLedger::open(&project.ledger_path, project_id)?;
    MarkdownProjector::new(brain_home.join("vault")).rebuild_project(&ledger, project_id)
}

pub fn rebuild_basic_memory(brain_home: &Path, project_id: ProjectId) -> Result<BasicMemoryReport> {
    rebuild_basic_memory_with(brain_home, project_id, &ProcessBasicMemoryCli::default())
}

pub fn rebuild_basic_memory_with(
    brain_home: &Path,
    project_id: ProjectId,
    cli: &dyn BasicMemoryCli,
) -> Result<BasicMemoryReport> {
    let projection = rebuild_markdown(brain_home, project_id)?;
    Ok(BasicMemoryIndexer::new(cli).rebuild(&projection))
}

pub fn verify_projections(
    brain_home: &Path,
    project_id: ProjectId,
) -> Result<ProjectionVerification> {
    MarkdownProjector::new(brain_home.join("vault")).verify_project(project_id)
}
