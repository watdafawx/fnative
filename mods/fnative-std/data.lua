-- styles for the fnative-std library
local styles = data.raw["gui-style"].default

-- the frame that follows the cursor while dragging
styles.fstd_ghost = {
  type = "frame_style",
  parent = "tooltip_frame",
  padding = 4,
  horizontal_flow_style = { type = "horizontal_flow_style", vertical_align = "center", horizontal_spacing = 6 },
}

-- the corner a window is resized by
styles.fstd_resize_grip = {
  type = "empty_widget_style",
  parent = "draggable_space",
  width = 16,
  height = 16,
  left_margin = 0,
  right_margin = 0,
}
