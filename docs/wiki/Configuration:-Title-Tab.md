### Overview

<sup>Fork only. Not part of upstream niri.</sup>

The title tab is a small BeOS-style tab drawn by the compositor above the top-left corner of every window.
It shows the window title, a close button before it and a maximize (zoom) button after it.
Two further styles turn it into a full-width title bar in the manner of FM TOWNS (`style "towns"`) or the classic Macintosh (`style "mac"`); see [`style`](#style).

* Click the tab to focus the window.
* Drag the tab to move the window, exactly like dragging it with the move modifier.
* Click the close button to close the window.
* Click the maximize button to toggle the column between its current width and the full width of the screen.
  Floating windows have no column, so their tab has no maximize button.

The tab takes its colors from the focus ring (or the border, if the focus ring is off), so it blends with the rest of your frame without extra configuration.
Its rounded corners follow the window's `geometry-corner-radius`.
The frame's top-left corner is squared off under the tab, while the window keeps its own rounded corner; the gap between them is filled in the tab's color.

In a tabbed column (`toggle-column-tabbed-display`), the windows' tabs are laid out in a row along the top, like browser tabs, and niri's side tab indicator is hidden.
Clicking a tab in the row switches to that window.

The tab is **on by default**.
Set `off` to disable it and get stock niri rendering.

Here are the contents of this section at a glance, with the defaults:

```kdl
layout {
    title-tab {
        // off
        style "filled"
        height 24
        padding 10
        font "sans 10"
        max-width-proportion 0.6
        corner-radius-factor 1.0
        join-radius-factor 0.5
        // row-gap 4
        // fill-color "#181818"
        // bar-color "#8cbdef"
        // inactive-bar-color "#ffffff"
        close-button
        close-button-size 10
        maximize-button
        // active-color "#ffcb00"
        // inactive-color "#505050"
        // urgent-color "#9b0000"
        // active-gradient from="#ffcb00" to="#74c7ec" angle=135
        // active-text-color "#000000"
        // inactive-text-color "#ffffff"
        // urgent-text-color "#ffffff"
    }
}
```

### `off` and `on`

`off` disables the tab.
Windows are then laid out and drawn exactly as in upstream niri.

`on` is accepted too, for configs that want to be explicit.

### `style`

`"filled"` (the default) draws a solid tab in the frame color, with black or white text chosen by the background's brightness.

`"outline"` draws a frame-colored line around the tab and joins it to the focus ring with a rounded bend, with the interior in `fill-color`.
The title text uses the frame color.
Unfocused windows show only the fill, matching how the focus ring disappears on unfocused windows.

`"towns"` is a Towns OS-style title bar.
The tab is followed by a bar that runs to the frame's right edge, filled with `bar-color` and stroked on its top and right in the frame color.
The close button becomes a gridded box, and the maximize button moves to the bar's right end as an outward-arrows icon.
On unfocused windows the whole bar, tab included, is filled with `inactive-bar-color`.
By default the focused tab is white and the bar light blue, and unfocused bars are white.

`"mac"` is a classic Macintosh title bar.
It spans the window, is white in every state, and takes the frame color for its top and side lines, title, stripes and boxes.
The title is centered between a close box on the left and a zoom box on the right, and while the window is focused, stripes run across the bar around them.
Unfocused windows show only the white bar and the title; the boxes are hidden, and clicking where they were just focuses the window.

In the `"towns"` and `"mac"` styles the window's top corners are square under the bar, the bar's own outer corners are rounded, and the drop shadow covers the bar as well as the window.
Both draw their lines in the focus ring color, so with either style the ring's default `active-color` is black instead of the fork's BeOS yellow.
A ring color or gradient you set yourself is used as is.
In a tabbed column the tabs split the bar between them and share one frame-colored line where they meet.

Switching `style` is a live config change like any other: edit it, and the running session redraws.

### `height`

Height of the tab in logical pixels.
The window's tile grows by this much at the top, so the tab never overlaps the window contents.

### `padding`

Horizontal space between the tab's edge and its contents, in logical pixels.
The same value is used between the buttons and the title.

### `font`

A [Pango font description](https://docs.gtk.org/Pango/type_func.FontDescription.from_string.html) for the title text, for example `"sans bold 10"` or `"Inter Semi-Bold 11"`.
The size is in points and scales with the output.

### `max-width-proportion`

The widest the tab may be, as a fraction of the window width.
Longer titles are cut with an ellipsis.

### `corner-radius-factor`

Radius of the tab's top corners as a fraction of the window's `geometry-corner-radius`.
`1.0` matches the window; `0.5` is a tighter curve; `0` is square.

### `join-radius-factor`

Radius of the bend where the tab's right edge meets the top of the frame, as a fraction of the window's corner radius.
`0` gives a sharp corner.

### `row-gap`

Gap between tabs in a tabbed column, in logical pixels.
When unset, the gap equals the bend radius so the bends on neighbouring tabs don't collide.

### `fill-color`

Interior color for the `"outline"` style.
A transparent color shows the wallpaper through the tab.
Defaults to a dark gray.

### `bar-color` and `inactive-bar-color`

Colors for the `"towns"` style.
`bar-color` fills the bar to the right of the tab on the focused window; it defaults to a light blue (`#8cbdef`).
`inactive-bar-color` fills the whole bar, tab included, on unfocused windows; it defaults to white.
An explicit `inactive-color` takes precedence over `inactive-bar-color`.

### `close-button` and `close-button-size`

`close-button` draws a square outline button before the title; clicking it closes the window.
`close-button-size` is the square's side in logical pixels.
Set `close-button false` to hide it.

In the `"towns"` style the square holds a grid of lines, and in the `"mac"` style it is the close box, shown only on the focused window.

### `maximize-button`

Draws the BeOS zoom button (two overlapping squares) after the title.
Clicking it toggles the column between its current width and the full width of the screen (`maximize-column`).
It is not shown on floating windows.
Set `maximize-button false` to hide it everywhere.

In the `"towns"` style the button sits at the right end of the bar and is drawn as four outward arrows.
In the `"mac"` style it is the zoom box at the bar's right, shown only on the focused window.

### Colors

By default the tab uses the focus ring's `active-color`, `inactive-color` and `urgent-color`, including gradients, so it always matches the ring.
Override any of them here with the same syntax as the [focus ring](./Configuration:-Layout.md#focus-ring-and-border):

```kdl
layout {
    title-tab {
        active-color "#ffcb00"
        inactive-color "#3a3a3a"
        active-gradient from="#ffcb00" to="#74c7ec" angle=135 in="oklab"
    }
}
```

`active-text-color`, `inactive-text-color` and `urgent-text-color` set the title text color for each state.
When unset, the filled and towns styles pick black or white by background brightness, and the outline and mac styles use the frame color while a frame is drawn.

The mac style's bar, and the towns style's focused tab, default to white instead of the frame color; the colors above recolor them if you set them.
Lines, stripes and boxes always come from the frame, which is why the focus ring defaults to black for these two styles.
Set `active-color` in `focus-ring` to pick another line color.

### Notes

* The tab is hidden while a window is maximized (`maximize-window-to-edges`) or fullscreen, and comes back when it leaves that state.
* The drop shadow (`layout { shadow {} }`) covers the bar in the towns and mac styles. In the filled and outline styles it wraps the window but not the tab yet.
* There is no per-window rule to disable the tab yet; `off` is global.
* With Dank Material Shell or another tool that writes the focus ring colors, the tab follows the theme automatically because it inherits from the ring.
