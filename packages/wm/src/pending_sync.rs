use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::{
  models::{Container, Workspace},
  traits::CommonGetters,
};

/// An update, besides redraws and reorders, that the next sync applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PendingUpdate {
  /// Native focus should be reassigned to the WM's focused container.
  Focus,

  /// Window effects for the focused window should be updated.
  FocusedEffect,

  /// Window effects for all windows should be updated.
  AllEffects,

  /// The cursor should jump to the focused container (if enabled in user
  /// config).
  CursorJump,
}

#[derive(Debug, Default)]
pub struct PendingSync {
  /// Containers (and their descendants) that have a pending redraw.
  containers_to_redraw: HashMap<Uuid, Container>,

  /// Workspaces where z-order should be updated. Windows that match the
  /// focused window's state should be brought to the front.
  workspaces_to_reorder: Vec<Workspace>,

  /// Other updates queued for the next sync.
  updates: HashSet<PendingUpdate>,
}

impl PendingSync {
  pub fn has_changes(&self) -> bool {
    !self.containers_to_redraw.is_empty()
      || !self.workspaces_to_reorder.is_empty()
      || !self.updates.is_empty()
  }

  pub fn clear(&mut self) -> &mut Self {
    self.containers_to_redraw.clear();
    self.workspaces_to_reorder.clear();
    self.updates.clear();
    self
  }

  pub fn queue_container_to_redraw<T>(&mut self, container: T) -> &mut Self
  where
    T: Into<Container>,
  {
    let container: Container = container.into();
    self.containers_to_redraw.insert(container.id(), container);
    self
  }

  pub fn queue_containers_to_redraw<I, T>(
    &mut self,
    containers: I,
  ) -> &mut Self
  where
    I: IntoIterator<Item = T>,
    T: Into<Container>,
  {
    for container in containers {
      let container: Container = container.into();
      self.containers_to_redraw.insert(container.id(), container);
    }

    self
  }

  pub fn dequeue_container_from_redraw<T>(
    &mut self,
    container: T,
  ) -> &mut Self
  where
    T: Into<Container>,
  {
    self.containers_to_redraw.remove(&container.into().id());
    self
  }

  pub fn queue_workspace_to_reorder(
    &mut self,
    workspace: Workspace,
  ) -> &mut Self {
    self.workspaces_to_reorder.push(workspace);
    self
  }

  pub fn queue_focus_change(&mut self) -> &mut Self {
    self.updates.insert(PendingUpdate::Focus);
    self
  }

  pub fn queue_focused_effect_update(&mut self) -> &mut Self {
    self.updates.insert(PendingUpdate::FocusedEffect);
    self
  }

  pub fn queue_all_effects_update(&mut self) -> &mut Self {
    self.updates.insert(PendingUpdate::AllEffects);
    self
  }

  pub fn queue_cursor_jump(&mut self) -> &mut Self {
    self.updates.insert(PendingUpdate::CursorJump);
    self
  }

  pub fn needs_focus_update(&self) -> bool {
    self.updates.contains(&PendingUpdate::Focus)
  }

  pub fn needs_focused_effect_update(&self) -> bool {
    self.updates.contains(&PendingUpdate::FocusedEffect)
  }

  pub fn needs_all_effects_update(&self) -> bool {
    self.updates.contains(&PendingUpdate::AllEffects)
  }

  pub fn needs_cursor_jump(&self) -> bool {
    self.updates.contains(&PendingUpdate::CursorJump)
  }

  pub fn containers_to_redraw(&self) -> &HashMap<Uuid, Container> {
    &self.containers_to_redraw
  }

  pub fn workspaces_to_reorder(&self) -> &Vec<Workspace> {
    &self.workspaces_to_reorder
  }
}
