// Copyright (C) 2023 Quickwit, Inc.
//
// Quickwit is offered under the AGPL v3.0 and as commercial software.
// For commercial licensing, contact us at hello@quickwit.io.
//
// AGPL:
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <http://www.gnu.org/licenses/>.

use std::io;

use anyhow::anyhow;
use quickwit_actors::AskError;
use thiserror;

use crate::{IndexUid, ServiceError, ServiceErrorCode};

#[path = "../codegen/quickwit/quickwit.indexing.rs"]
mod codegen;

pub use codegen::*;

pub type Result<T> = std::result::Result<T, IndexingError>;

#[derive(Debug, thiserror::Error)]
pub enum IndexingError {
    #[error("Indexing pipeline `{index_id}` for source `{source_id}` does not exist.")]
    MissingPipeline { index_id: String, source_id: String },
    #[error(
        "Pipeline #{pipeline_ord} for index `{index_id}` and source `{source_id}` already exists."
    )]
    PipelineAlreadyExists {
        index_id: String,
        source_id: String,
        pipeline_ord: usize,
    },
    #[error("I/O Error `{0}`.")]
    Io(io::Error),
    #[error("Invalid params `{0}`.")]
    InvalidParams(anyhow::Error),
    #[error("Spanw pipelines errors `{pipeline_ids:?}`.")]
    SpawnPipelinesError {
        pipeline_ids: Vec<IndexingPipelineId>,
    },
    #[error("A metastore error occurred: {0}.")]
    MetastoreError(String),
    #[error("A storage resolver error occurred: {0}.")]
    StorageResolverError(String),
    #[error("An internal error occurred: {0}.")]
    Internal(String),
    #[error("The ingest service is unavailable.")]
    Unavailable,
}

impl From<IndexingError> for tonic::Status {
    fn from(error: IndexingError) -> Self {
        match error {
            IndexingError::MissingPipeline {
                index_id,
                source_id,
            } => tonic::Status::not_found(format!("Missing pipeline {index_id}/{source_id}")),
            IndexingError::PipelineAlreadyExists {
                index_id,
                source_id,
                pipeline_ord,
            } => tonic::Status::already_exists(format!(
                "Pipeline {index_id}/{source_id} {pipeline_ord} already exists "
            )),
            IndexingError::Io(error) => tonic::Status::internal(error.to_string()),
            IndexingError::InvalidParams(error) => {
                tonic::Status::invalid_argument(error.to_string())
            }
            IndexingError::SpawnPipelinesError { pipeline_ids } => {
                tonic::Status::internal(format!("Error spawning pipelines {:?}", pipeline_ids))
            }
            IndexingError::Internal(string) => tonic::Status::internal(string),
            IndexingError::MetastoreError(string) => tonic::Status::internal(string),
            IndexingError::StorageResolverError(string) => tonic::Status::internal(string),
            IndexingError::Unavailable => {
                tonic::Status::unavailable("Indexing service is unavailable.")
            }
        }
    }
}

impl From<tonic::Status> for IndexingError {
    fn from(status: tonic::Status) -> Self {
        match status.code() {
            tonic::Code::InvalidArgument => {
                IndexingError::InvalidParams(anyhow!(status.message().to_string()))
            }
            tonic::Code::NotFound => IndexingError::MissingPipeline {
                index_id: "".to_string(),
                source_id: "".to_string(),
            },
            tonic::Code::AlreadyExists => IndexingError::PipelineAlreadyExists {
                index_id: "".to_string(),
                source_id: "".to_string(),
                pipeline_ord: 0,
            },
            tonic::Code::Unavailable => IndexingError::Unavailable,
            _ => IndexingError::InvalidParams(anyhow!(status.message().to_string())),
        }
    }
}

impl ServiceError for IndexingError {
    fn status_code(&self) -> ServiceErrorCode {
        match self {
            Self::MissingPipeline { .. } => ServiceErrorCode::NotFound,
            Self::PipelineAlreadyExists { .. } => ServiceErrorCode::BadRequest,
            Self::InvalidParams(_) => ServiceErrorCode::BadRequest,
            Self::SpawnPipelinesError { .. } => ServiceErrorCode::Internal,
            Self::Io(_) => ServiceErrorCode::Internal,
            Self::Internal(_) => ServiceErrorCode::Internal,
            Self::MetastoreError(_) => ServiceErrorCode::Internal,
            Self::StorageResolverError(_) => ServiceErrorCode::Internal,
            Self::Unavailable => ServiceErrorCode::Unavailable,
        }
    }
}

impl From<AskError<IndexingError>> for IndexingError {
    fn from(error: AskError<IndexingError>) -> Self {
        match error {
            AskError::ErrorReply(error) => error,
            AskError::MessageNotDelivered => IndexingError::Unavailable,
            AskError::ProcessMessageError => IndexingError::Internal(
                "An error occurred while processing the request".to_string(),
            ),
        }
    }
}

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub struct IndexingPipelineId {
    pub index_uid: IndexUid,
    pub source_id: String,
    pub node_id: String,
    pub pipeline_ord: usize,
}

impl ToString for IndexingTask {
    fn to_string(&self) -> String {
        format!("{}:{}", self.index_uid, self.source_id)
    }
}

impl TryFrom<&str> for IndexingTask {
    type Error = anyhow::Error;

    fn try_from(index_task_str: &str) -> anyhow::Result<IndexingTask> {
        let mut iter = index_task_str.rsplit(':');
        let source_id = iter.next().ok_or_else(|| {
            anyhow!(
                "Invalid index task format, cannot find source_id in `{}`",
                index_task_str
            )
        })?;
        let part1 = iter.next().ok_or_else(|| {
            anyhow!(
                "Invalid index task format, cannot find index_id in `{}`",
                index_task_str
            )
        })?;
        if let Some(part2) = iter.next() {
            Ok(IndexingTask {
                index_uid: format!("{part2}:{part1}"),
                source_id: source_id.to_string(),
            })
        } else {
            Ok(IndexingTask {
                index_uid: part1.to_string(),
                source_id: source_id.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indexing_task_serialization() {
        let original = IndexingTask {
            index_uid: "test-index:123456".to_string(),
            source_id: "test-source".to_string(),
        };

        let serialized = original.to_string();
        let deserialized: IndexingTask = serialized.as_str().try_into().unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_indexing_task_serialization_bwc() {
        assert_eq!(
            IndexingTask::try_from("foo:bar").unwrap(),
            IndexingTask {
                index_uid: "foo".to_string(),
                source_id: "bar".to_string(),
            }
        );
    }

    #[test]
    fn test_indexing_task_serialization_errors() {
        assert_eq!(
            "Invalid index task format, cannot find index_id in ``",
            IndexingTask::try_from("").unwrap_err().to_string()
        );
        assert_eq!(
            "Invalid index task format, cannot find index_id in `foo`",
            IndexingTask::try_from("foo").unwrap_err().to_string()
        );
    }
}
