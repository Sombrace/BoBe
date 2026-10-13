# bobe_text

Assemble `display.text` messages into caller-owned UTF-8 storage without a GUI
dependency or allocation:

```c
char text[256];
bobe_text_view_t view;
bobe_text_view_init(&view, text, sizeof(text));

/* In the single task that owns this view: */
if (bobe_text_view_apply(&view, message) == BOBE_TEXT_APPLIED) {
    render_plain_text(view.buffer);
}
```

The oldest complete code points are dropped when storage fills. `truncated`
records that condition. Sequence gaps and duplicate/foreign request chunks have
distinct results. A new request's clear frame replaces the view. Buffers must
remain alive; calls for a view must be serialized by its owning task.

The renderer is supplied by the application: LVGL, an OLED driver, a character
LCD, serial output, or another board-specific UI.
