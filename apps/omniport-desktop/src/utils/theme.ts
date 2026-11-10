/**
 * Antigravity Semantic Theme Tokens & Serial Monitor Palette
 */

export interface MonitorTheme {
  background: string;
  cardBackground: string;
  borderColor: string;
  headerBackground: string;
  headerText: string;
  indexText: string;
  timestampText: string;
  txText: string;
  rxText: string;
  errorText: string;
  hexText: string;
  asciiText: string;
  asciiUnprintable: string;
  selectionFill: string;
  hoverFill: string;
  pinHigh: string;
  pinLow: string;
  pinEdge: string;
  pinText: string;
  /** Topology graph node body / node header surfaces. */
  nodeBackground: string;
  nodeHeaderBackground: string;
}

export const darkTheme: MonitorTheme = {
  background: "#0d1117",
  cardBackground: "#161b22",
  borderColor: "#30363d",
  headerBackground: "#161b22",
  headerText: "#8b949e",
  indexText: "#6e7681",
  timestampText: "#8b949e",
  txText: "#58a6ff", // Bright Blue
  rxText: "#3fb950", // Fresh Green
  errorText: "#f85149", // Vivid Red
  hexText: "#c9d1d9",
  asciiText: "#e6edf3",
  asciiUnprintable: "#484f58",
  selectionFill: "rgba(56, 139, 253, 0.25)",
  hoverFill: "rgba(110, 118, 129, 0.12)",
  pinHigh: "#2ea043", // Green HIGH
  pinLow: "#21262d", // Muted LOW
  pinEdge: "#58a6ff", // Transition pulse
  pinText: "#e6edf3",
  nodeBackground: "#161b22",
  nodeHeaderBackground: "#21262d",
};

export const lightTheme: MonitorTheme = {
  background: "#ffffff",
  cardBackground: "#f6f8fa",
  borderColor: "#d0d7de",
  headerBackground: "#f6f8fa",
  headerText: "#57606a",
  indexText: "#6e7781",
  timestampText: "#57606a",
  txText: "#0969da",
  rxText: "#1a7f37",
  errorText: "#cf222e",
  hexText: "#24292f",
  asciiText: "#1f2328",
  asciiUnprintable: "#8c959f",
  selectionFill: "rgba(9, 105, 218, 0.15)",
  hoverFill: "rgba(234, 238, 242, 0.6)",
  pinHigh: "#1a7f37",
  pinLow: "#eaeef2",
  pinEdge: "#0969da",
  pinText: "#1f2328",
  nodeBackground: "#f6f8fa",
  nodeHeaderBackground: "#eaeef2",
};

export function getTheme(isDark = true): MonitorTheme {
  return isDark ? darkTheme : lightTheme;
}
