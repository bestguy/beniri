### Overview

<sup>Fork only. Not part of upstream niri.</sup>

The title tab is a small BeOS-style tab drawn by the compositor above the top-left corner of every window.
It shows the window title, a close button before it and a maximize (zoom) button after it.

* Click the tab to focus the window.
* Drag the tab to move the window, exactly like dragging it with the move modifier.
* Click the close button to close the window.
* Click the maximize button to toggle the column between its current width and the full width of the screen.

The tab takes its colors from the focus ring (or the border, if the focus ring is off), so it blends with the rest of your frame without extra configuration.
Its rounded corners follow the window's `geometry-corner-radius`.

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

### `close-button` and `close-button-size`

`close-button` draws a square outline button before the title; clicking it closes the window.
`close-button-size` is the square's side in logical pixels.
Set `close-button false` to hide it.

### `maximize-button`

Draws the BeOS zoom button (two overlapping squares) after the title.
Clicking it toggles the column between its current width and the full width of the screen (`maximize-column`).
Set `maximize-button false` to hide it.

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
When unset, the filled style picks black or white by background brightness and the outline style uses the frame color.

### Notes

* The tab is hidden while a window is maximized (`maximize-window-to-edges`) or fullscreen, and comes back when it leaves that state.
* The drop shadow (`layout { shadow {} }`) wraps the window but not the tab yet.
* There is no per-window rule to disable the tab yet; `off` is global.
* With Dank Material Shell or another tool that writes the focus ring colors, the tab follows the theme automatically because it inherits from the ring.
