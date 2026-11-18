/**
 * Copy text out of the Tauri WebView.
 *
 * `navigator.clipboard` needs a secure context and a WebView build that grants
 * the permission. On Windows WebView2 it can be missing or rejected, which is
 * why `Copy diagnostics` reported a failure while the report itself was fine.
 * The DOM `copy` command is the fallback; when even that is blocked the caller
 * shows the text so it can be selected by hand.
 */
export async function copyText(text: string): Promise<boolean> {
  try {
    if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // Denied, not available, or not allowed in this context: try the DOM path.
  }
  return legacyCopy(text);
}

/** Off-screen, read-only textarea + `document.execCommand("copy")`. */
function legacyCopy(text: string): boolean {
  if (typeof document === "undefined" || !document.body) {
    return false;
  }
  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  area.setAttribute("aria-hidden", "true");
  area.style.position = "fixed";
  area.style.top = "-1000px";
  area.style.left = "-1000px";
  area.style.opacity = "0";
  document.body.appendChild(area);
  const selection = document.getSelection();
  const previous = selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null;
  let copied = false;
  try {
    area.select();
    copied = document.execCommand("copy");
  } catch {
    copied = false;
  } finally {
    document.body.removeChild(area);
    if (previous && selection) {
      selection.removeAllRanges();
      selection.addRange(previous);
    }
  }
  return copied;
}
