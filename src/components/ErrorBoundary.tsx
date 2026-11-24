import React from "react";

interface Props {
  children: React.ReactNode;
  /** Short name of what failed, shown in the fallback. */
  label?: string;
  /** Full-window fallback (app shell) instead of an in-page panel. */
  fullScreen?: boolean;
}

interface State {
  error: Error | null;
}

/**
 * Keeps one failing page (a render bug, a lazy chunk that failed to load)
 * from blanking the whole window, which until now only a restart recovered.
 */
export class ErrorBoundary extends React.Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error(`[${this.props.label ?? "app"}] render failed`, error, info.componentStack);
  }

  private retry = () => {
    if (this.props.fullScreen) {
      window.location.reload();
      return;
    }
    this.setState({ error: null });
  };

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div
        className={`${
          this.props.fullScreen ? "h-screen w-screen" : "h-full w-full"
        } flex items-center justify-center p-6 bg-transparent`}
      >
        <div className="max-w-md w-full rounded-2xl border border-rose-500/30 bg-rose-500/5 p-5 text-sm">
          <p className="font-semibold text-rose-400">
            {this.props.label ? `${this.props.label} failed to load` : "Something went wrong"}
          </p>
          <p className="mt-2 text-zinc-400 break-words">{error.message || String(error)}</p>
          <button
            type="button"
            onClick={this.retry}
            className="mt-4 px-3 py-1.5 rounded-lg bg-zinc-800 hover:bg-zinc-700 text-zinc-100 text-xs font-medium"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }
}
