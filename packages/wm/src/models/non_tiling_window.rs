use std::{
  cell::{Ref, RefCell, RefMut},
  collections::VecDeque,
  rc::Rc,
};

use anyhow::Context;
use uuid::Uuid;
use wm_common::{
  ActiveDrag, ContainerDto, DisplayState, GapsConfig, WindowDto,
  WindowRuleConfig, WindowState,
};
use wm_platform::{NativeWindow, Rect, RectDelta};

use crate::{
  impl_common_getters, impl_container_debug, impl_window_getters,
  models::{
    Container, DirectionContainer, InsertionTarget,
    NativeWindowProperties, TilingContainer, TilingWindow,
    WindowContainer, WindowInit,
  },
  traits::{CommonGetters, PositionGetters, WindowGetters},
};

#[derive(Clone)]
pub struct NonTilingWindow(Rc<RefCell<NonTilingWindowInner>>);

struct NonTilingWindowInner {
  id: Uuid,
  parent: Option<Container>,
  children: VecDeque<Container>,
  child_focus_order: VecDeque<Uuid>,
  native: NativeWindow,
  native_properties: NativeWindowProperties,
  state: WindowState,
  prev_state: Option<WindowState>,
  insertion_target: Option<InsertionTarget>,
  display_state: DisplayState,
  border_delta: RectDelta,
  has_pending_dpi_adjustment: bool,
  floating_placement: Rect,
  has_custom_floating_placement: bool,
  done_window_rules: Vec<WindowRuleConfig>,
  active_drag: Option<ActiveDrag>,
}

impl NonTilingWindow {
  pub fn new(
    init: WindowInit,
    state: WindowState,
    insertion_target: Option<InsertionTarget>,
  ) -> Self {
    let window = NonTilingWindowInner {
      id: init.id.unwrap_or_else(Uuid::new_v4),
      parent: None,
      children: VecDeque::new(),
      child_focus_order: VecDeque::new(),
      native: init.native,
      native_properties: init.properties,
      state,
      prev_state: init.prev_state,
      insertion_target,
      display_state: DisplayState::Shown,
      border_delta: init.border_delta,
      has_pending_dpi_adjustment: false,
      floating_placement: init.floating_placement,
      has_custom_floating_placement: init.has_custom_floating_placement,
      done_window_rules: init.done_window_rules,
      active_drag: init.active_drag,
    };

    Self(Rc::new(RefCell::new(window)))
  }

  pub fn insertion_target(&self) -> Option<InsertionTarget> {
    self.0.borrow().insertion_target.clone()
  }

  pub fn set_insertion_target(
    &self,
    insertion_target: Option<InsertionTarget>,
  ) {
    self.0.borrow_mut().insertion_target = insertion_target;
  }

  pub fn to_tiling(&self, gaps_config: GapsConfig) -> TilingWindow {
    let prev_state = if self.active_drag().is_some() {
      self.prev_state()
    } else {
      Some(self.state())
    };

    TilingWindow::new(
      WindowInit {
        id: Some(self.id()),
        native: self.native().clone(),
        properties: self.native_properties().clone(),
        prev_state,
        border_delta: self.border_delta(),
        floating_placement: self.floating_placement(),
        has_custom_floating_placement: self
          .has_custom_floating_placement(),
        done_window_rules: self.done_window_rules(),
        active_drag: self.active_drag(),
      },
      gaps_config,
    )
  }

  pub fn to_dto(&self) -> anyhow::Result<ContainerDto> {
    let rect = self.to_rect()?;

    Ok(ContainerDto::Window(WindowDto {
      id: self.id(),
      parent_id: self.parent().map(|parent| parent.id()),
      has_focus: self.has_focus(None),
      tiling_size: None,
      width: rect.width(),
      height: rect.height(),
      x: rect.x(),
      y: rect.y(),
      state: self.state(),
      prev_state: self.prev_state(),
      display_state: self.display_state(),
      border_delta: self.border_delta(),
      floating_placement: self.floating_placement(),
      handle: self.native().id().0,
      title: self.native_properties().title,
      #[cfg(target_os = "windows")]
      class_name: self.native_properties().class_name,
      process_name: self.native_properties().process_name,
      active_drag: self.active_drag(),
    }))
  }
}

impl_container_debug!(NonTilingWindow);
impl_common_getters!(NonTilingWindow);
impl_window_getters!(NonTilingWindow);

impl PositionGetters for NonTilingWindow {
  fn to_rect(&self) -> anyhow::Result<Rect> {
    match self.state() {
      WindowState::Fullscreen(_) => {
        let monitor = self.monitor().context("No monitor.")?;

        #[cfg(target_os = "windows")]
        {
          monitor.to_rect()
        }
      }
      _ => Ok(self.floating_placement()),
    }
  }
}
