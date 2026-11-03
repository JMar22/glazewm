use std::{
  cell::{Ref, RefCell, RefMut},
  collections::VecDeque,
  rc::Rc,
};

use anyhow::Context;
use uuid::Uuid;
use wm_common::{
  ActiveDrag, ContainerDto, DisplayState, GapsConfig, TilingDirection,
  WindowDto, WindowRuleConfig, WindowState,
};
use wm_platform::{NativeWindow, Rect, RectDelta};

use crate::{
  impl_common_getters, impl_container_debug,
  impl_position_getters_as_resizable, impl_tiling_size_getters,
  impl_window_getters,
  models::{
    Container, DirectionContainer, InsertionTarget,
    NativeWindowProperties, NonTilingWindow, TilingContainer,
    WindowContainer, WindowInit,
  },
  traits::{
    CommonGetters, PositionGetters, TilingDirectionGetters,
    TilingSizeGetters, WindowGetters,
  },
};

#[derive(Clone)]
pub struct TilingWindow(Rc<RefCell<TilingWindowInner>>);

struct TilingWindowInner {
  id: Uuid,
  parent: Option<Container>,
  children: VecDeque<Container>,
  child_focus_order: VecDeque<Uuid>,
  tiling_size: f32,
  native: NativeWindow,
  native_properties: NativeWindowProperties,
  state: WindowState,
  prev_state: Option<WindowState>,
  display_state: DisplayState,
  border_delta: RectDelta,
  has_pending_dpi_adjustment: bool,
  floating_placement: Rect,
  has_custom_floating_placement: bool,
  gaps_config: GapsConfig,
  done_window_rules: Vec<WindowRuleConfig>,
  active_drag: Option<ActiveDrag>,
}

impl TilingWindow {
  pub fn new(init: WindowInit, gaps_config: GapsConfig) -> Self {
    let window = TilingWindowInner {
      id: init.id.unwrap_or_else(Uuid::new_v4),
      parent: None,
      children: VecDeque::new(),
      child_focus_order: VecDeque::new(),
      tiling_size: 1.0,
      native: init.native,
      native_properties: init.properties,
      state: WindowState::Tiling,
      prev_state: init.prev_state,
      display_state: DisplayState::Shown,
      border_delta: init.border_delta,
      has_pending_dpi_adjustment: false,
      floating_placement: init.floating_placement,
      has_custom_floating_placement: init.has_custom_floating_placement,
      gaps_config,
      done_window_rules: init.done_window_rules,
      active_drag: init.active_drag,
    };

    Self(Rc::new(RefCell::new(window)))
  }

  pub fn to_non_tiling(
    &self,
    state: WindowState,
    insertion_target: Option<InsertionTarget>,
  ) -> NonTilingWindow {
    NonTilingWindow::new(
      WindowInit {
        id: Some(self.id()),
        native: self.native().clone(),
        properties: self.native_properties().clone(),
        prev_state: Some(WindowState::Tiling),
        border_delta: self.border_delta(),
        floating_placement: self.floating_placement(),
        has_custom_floating_placement: self
          .has_custom_floating_placement(),
        done_window_rules: self.done_window_rules(),
        active_drag: self.active_drag(),
      },
      state,
      insertion_target,
    )
  }

  pub fn to_dto(&self) -> anyhow::Result<ContainerDto> {
    let rect = self.to_rect()?;

    Ok(ContainerDto::Window(WindowDto {
      id: self.id(),
      parent_id: self.parent().map(|parent| parent.id()),
      has_focus: self.has_focus(None),
      tiling_size: Some(self.tiling_size()),
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

impl_container_debug!(TilingWindow);
impl_common_getters!(TilingWindow);
impl_tiling_size_getters!(TilingWindow);
impl_position_getters_as_resizable!(TilingWindow);
impl_window_getters!(TilingWindow);
