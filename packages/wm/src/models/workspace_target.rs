use wm_common::RelativeWorkspace;
use wm_platform::Direction;

pub enum WorkspaceTarget {
  Name(String),
  Recent,
  NextActive,
  PreviousActive,
  NextActiveInMonitor,
  PreviousActiveInMonitor,
  Next,
  Previous,
  Direction(Direction),
}

impl From<RelativeWorkspace> for WorkspaceTarget {
  fn from(target: RelativeWorkspace) -> Self {
    match target {
      RelativeWorkspace::NextActive => Self::NextActive,
      RelativeWorkspace::PrevActive => Self::PreviousActive,
      RelativeWorkspace::Next => Self::Next,
      RelativeWorkspace::Prev => Self::Previous,
      RelativeWorkspace::NextActiveOnMonitor => Self::NextActiveInMonitor,
      RelativeWorkspace::PrevActiveOnMonitor => {
        Self::PreviousActiveInMonitor
      }
      RelativeWorkspace::Recent => Self::Recent,
    }
  }
}
