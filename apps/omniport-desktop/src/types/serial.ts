/**
 * OmniPort Serial Data Types & Bitfield Constants
 * Aligned with docs/ICD.md and omniport-core
 */

export const PinFlags = {
  RTS: 0x01,   // Bit 0: Request To Send
  CTS: 0x02,   // Bit 1: Clear To Send
  DTR: 0x04,   // Bit 2: Data Terminal Ready
  DSR: 0x08,   // Bit 3: Data Set Ready
  DCD: 0x10,   // Bit 4: Data Carrier Detect
  RI: 0x20,    // Bit 5: Ring Indicator
  BREAK: 0x40, // Bit 6: Line Break Condition
} as const;

export const FrameFlags = {
  DIR_TX: 0x01,       // Bit 0: 1 = TX (Host -> Device), 0 = RX (Device -> Host)
  ERR_PARITY: 0x02,   // Bit 1: Parity error
  ERR_FRAMING: 0x04,  // Bit 2: Framing error (stop bit missing)
  ERR_OVERRUN: 0x08,  // Bit 3: Buffer overrun
  ERR_CRC: 0x10,      // Bit 4: CRC mismatch
  FAULT_INJECT: 0x20, // Bit 5: Artificial fault injected
  PIN_CHANGE: 0x40,   // Bit 6: Pin state change event
} as const;

export interface PinStateSnapshot {
  rts: boolean;
  cts: boolean;
  dtr: boolean;
  dsr: boolean;
  dcd: boolean;
  ri: boolean;
  break: boolean;
}

export function extractPinStates(pins: number): PinStateSnapshot {
  return {
    rts: (pins & PinFlags.RTS) !== 0,
    cts: (pins & PinFlags.CTS) !== 0,
    dtr: (pins & PinFlags.DTR) !== 0,
    dsr: (pins & PinFlags.DSR) !== 0,
    dcd: (pins & PinFlags.DCD) !== 0,
    ri: (pins & PinFlags.RI) !== 0,
    break: (pins & PinFlags.BREAK) !== 0,
  };
}

export interface ViewportRowData {
  rowIndex: number;
  timestampNs: bigint;
  endpointId: number;
  flags: number;
  pins: number;
  payload: Uint8Array;
}
