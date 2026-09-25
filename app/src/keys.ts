// Window-wide keyboard shortcuts (Ctrl+Z, Delete, Esc…) belong to the view on screen. The 3D
// view stays mounted behind the diagram tabs, so without this its shortcuts fired there too.

/** Whether a window keydown is a shortcut for a view: the view is shown and no field has focus. */
export function isViewKey(ev: KeyboardEvent, shown: boolean): boolean {
  return shown && !(ev.target instanceof Element && ev.target.closest("input, textarea, select"));
}
