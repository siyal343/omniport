import React, { Component, type ErrorInfo, type ReactNode } from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App.tsx";

interface ErrorBoundaryProps {
  children: ReactNode;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
  errorInfo: ErrorInfo | null;
}

class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = { hasError: false, error: null, errorInfo: null };
  }

  static getDerivedStateFromError(error: Error): Partial<ErrorBoundaryState> {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    console.error("UI Uncaught Error:", error, errorInfo);
    this.setState({ error, errorInfo });
  }

  render() {
    if (this.state.hasError) {
      return (
        <div
          style={{
            backgroundColor: "#0d1117",
            color: "#f85149",
            padding: 24,
            fontFamily: "monospace",
            height: "100vh",
            overflow: "auto",
          }}
        >
          <h2 style={{ marginTop: 0 }}>OmniPort UI Error</h2>
          <pre
            style={{
              color: "#e6edf3",
              backgroundColor: "#161b22",
              padding: 16,
              borderRadius: 6,
              whiteSpace: "pre-wrap",
            }}
          >
            {this.state.error?.toString()}
          </pre>
          <pre style={{ color: "#8b949e", fontSize: 12, whiteSpace: "pre-wrap" }}>
            {this.state.errorInfo?.componentStack}
          </pre>
          <button
            onClick={() => window.location.reload()}
            style={{
              padding: "8px 16px",
              backgroundColor: "#238636",
              color: "#fff",
              border: "none",
              borderRadius: 6,
              cursor: "pointer",
              marginTop: 16,
              fontWeight: 600,
            }}
          >
            Reload Window
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

// Global unhandled error handlers
window.addEventListener("error", (event) => {
  console.error("Global Window Error:", event.error || event.message);
});
window.addEventListener("unhandledrejection", (event) => {
  console.error("Unhandled Promise Rejection:", event.reason);
});

const rootElement = document.getElementById("root");
if (rootElement) {
  ReactDOM.createRoot(rootElement).render(
    <React.StrictMode>
      <ErrorBoundary>
        <App />
      </ErrorBoundary>
    </React.StrictMode>
  );
}
