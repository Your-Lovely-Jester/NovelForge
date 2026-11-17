import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

/**
 * A render error used to leave a white window with no way out. Keep the shell
 * visible, show the message, and offer a reload plus a copyable report.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("NovelForge render error", error, info.componentStack);
  }

  private copyReport = async () => {
    const error = this.state.error;
    if (!error) return;
    const report = [
      "NovelForge UI error",
      `message: ${error.message}`,
      `stack: ${error.stack ?? "—"}`,
    ].join("\n");
    try {
      await navigator.clipboard.writeText(report);
    } catch {
      // Clipboard may be unavailable; the stack is still on screen.
    }
  };

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="flex min-h-screen items-center justify-center bg-[#08090a] px-6 text-parchment-300">
        <div className="panel w-full max-w-xl p-6">
          <p className="text-[11px] font-semibold uppercase tracking-[0.2em] text-vermilion-300">
            Interface error
          </p>
          <h1 className="mt-2 font-display text-[26px] text-parchment-50">
            NovelForge hit an unexpected error
          </h1>
          <p className="mt-2 text-[14px] leading-relaxed text-parchment-400">
            Downloads already running may still finish. Reload the window, then
            copy diagnostics from About if the problem returns.
          </p>
          <pre className="mt-4 max-h-48 overflow-auto border border-parchment-100/10 bg-black/40 p-3 font-mono text-[11px] leading-relaxed text-parchment-500">
            {this.state.error.message}
            {this.state.error.stack ? `\n\n${this.state.error.stack}` : ""}
          </pre>
          <div className="mt-4 flex flex-wrap gap-2">
            <button
              type="button"
              className="btn-primary"
              onClick={() => window.location.reload()}
            >
              Reload window
            </button>
            <button
              type="button"
              className="btn-secondary"
              onClick={() => void this.copyReport()}
            >
              Copy error
            </button>
          </div>
        </div>
      </div>
    );
  }
}
