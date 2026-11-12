// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Progress as owned data. [`Event`] borrows from the plan, so it can't
//! leave the thread that runs it; an [`Update`] can, for example to a
//! window. A client already holds the plan, so an update names its task by
//! index.

use crate::error::Error;
use crate::exec::{Event, Outcome};

/// How a finished task ended.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub enum Finish {
    Done(Outcome),
    Failed(Error),
}

/// An owned copy of an [`Event`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", rename_all = "camelCase"))]
pub enum Update {
    Started {
        index: usize,
        total: usize,
    },
    Progress {
        index: usize,
        total: usize,
        fraction: f32,
    },
    Finished {
        index: usize,
        total: usize,
        result: Finish,
    },
}

impl From<&Event<'_>> for Update {
    fn from(event: &Event<'_>) -> Update {
        match *event {
            Event::Started { index, total, .. } => Update::Started { index, total },
            Event::Progress {
                index,
                total,
                fraction,
                ..
            } => Update::Progress {
                index,
                total,
                fraction,
            },
            Event::Finished {
                index,
                total,
                result,
                ..
            } => Update::Finished {
                index,
                total,
                result: match result {
                    Ok(outcome) => Finish::Done(outcome.clone()),
                    Err(e) => Finish::Failed(e.clone()),
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::format::FormatId;
    use crate::job::PlannedTask;
    use crate::registry::Backend;

    #[test]
    fn update_from_event_copies_the_fields() {
        let task = PlannedTask {
            inputs: vec!["a.png".into()],
            from: vec![FormatId::Png],
            to: Some(FormatId::Webp),
            backend: Backend::Image,
            output: "a.webp".into(),
        };
        let started = Event::Started {
            index: 1,
            total: 3,
            task: &task,
        };
        assert_eq!(
            Update::from(&started),
            Update::Started { index: 1, total: 3 }
        );
        let progress = Event::Progress {
            index: 1,
            total: 3,
            task: &task,
            fraction: 0.25,
        };
        assert_eq!(
            Update::from(&progress),
            Update::Progress {
                index: 1,
                total: 3,
                fraction: 0.25
            }
        );
        let ok = Ok(Outcome::Written("a.webp".into()));
        let done = Event::Finished {
            index: 1,
            total: 3,
            task: &task,
            result: &ok,
        };
        assert_eq!(
            Update::from(&done),
            Update::Finished {
                index: 1,
                total: 3,
                result: Finish::Done(Outcome::Written("a.webp".into()))
            }
        );
        let err = Err(Error::new(ErrorCode::Cancelled, "cancelled"));
        let failed = Event::Finished {
            index: 2,
            total: 3,
            task: &task,
            result: &err,
        };
        assert_eq!(
            Update::from(&failed),
            Update::Finished {
                index: 2,
                total: 3,
                result: Finish::Failed(Error::new(ErrorCode::Cancelled, "cancelled"))
            }
        );
    }
}
