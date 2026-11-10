/**
 * Shape of the Tauri `capture_status` command response.
 *
 * `capture_status` lets the GUI distinguish host-seeded demo frames from a
 * live daemon capture instead of inferring it from frame content. The parser is
 * pure so it can be unit-tested and so a missing/older host command degrades to
 * `null` ("unknown host") rather than throwing.
 */

export interface HostCaptureStatus {
  demoMode: boolean;
  totalPackets: number;
  droppedFrames: number;
}

export function parseCaptureStatus(raw: unknown): HostCaptureStatus | null {
  if (typeof raw !== "object" || raw === null) return null;
  const record = raw as Record<string, unknown>;
  const demoMode = record.demo_mode;
  const totalPackets = record.total_packets;
  const droppedFrames = record.dropped_frames;
  if (
    typeof demoMode !== "boolean" ||
    typeof totalPackets !== "number" ||
    typeof droppedFrames !== "number"
  ) {
    return null;
  }
  return { demoMode, totalPackets, droppedFrames };
}
