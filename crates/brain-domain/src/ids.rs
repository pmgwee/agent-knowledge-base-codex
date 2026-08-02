#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(transparent)]
pub struct ProjectId(pub uuid::Uuid);

impl ProjectId {
    pub(crate) fn new() -> Self {
        Self(uuid::Uuid::now_v7())
    }

    pub(crate) fn unregistered() -> Self {
        Self(uuid::Uuid::nil())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(transparent)]
pub struct WorktreeId(pub uuid::Uuid);

impl WorktreeId {
    pub(crate) fn new() -> Self {
        Self(uuid::Uuid::now_v7())
    }

    pub(crate) fn unregistered() -> Self {
        Self(uuid::Uuid::nil())
    }
}
