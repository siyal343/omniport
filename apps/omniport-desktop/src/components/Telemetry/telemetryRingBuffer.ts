/**
 * Zero-Allocation Sliding Window Ring Buffer for 60 Hz Telemetry Strip Charts
 * Aligned with uPlot typed array layout [xSeries, ySeries0, ySeries1, ...].
 */

export class TelemetryRingBuffer {
  private readonly capacity: number;
  private length: number = 0;
  private head: number = 0;

  // Ring storage buffers
  private readonly timestamps: Float64Array;
  private readonly packetRate: Float64Array;
  private readonly throughputKbps: Float64Array;
  private readonly jitterMs: Float64Array;
  private readonly crcErrors: Float64Array;

  // Linearized export buffers (reused across 60 Hz frames, zero GC)
  private readonly exportTime: Float64Array;
  private readonly exportRate: Float64Array;
  private readonly exportThroughput: Float64Array;
  private readonly exportJitter: Float64Array;
  private readonly exportCrc: Float64Array;

  constructor(capacity = 600) {
    this.capacity = capacity;
    this.timestamps = new Float64Array(capacity);
    this.packetRate = new Float64Array(capacity);
    this.throughputKbps = new Float64Array(capacity);
    this.jitterMs = new Float64Array(capacity);
    this.crcErrors = new Float64Array(capacity);

    this.exportTime = new Float64Array(capacity);
    this.exportRate = new Float64Array(capacity);
    this.exportThroughput = new Float64Array(capacity);
    this.exportJitter = new Float64Array(capacity);
    this.exportCrc = new Float64Array(capacity);
  }

  /**
   * Appends a new 60 Hz telemetry sample without heap allocations
   */
  public push(
    timestampSec: number,
    rate: number,
    throughput: number,
    jitter: number,
    crc: number
  ): void {
    const idx = (this.head + this.length) % this.capacity;
    this.timestamps[idx] = timestampSec;
    this.packetRate[idx] = rate;
    this.throughputKbps[idx] = throughput;
    this.jitterMs[idx] = jitter;
    this.crcErrors[idx] = crc;

    if (this.length < this.capacity) {
      this.length++;
    } else {
      this.head = (this.head + 1) % this.capacity;
    }
  }

  public getLength(): number {
    return this.length;
  }

  public getCapacity(): number {
    return this.capacity;
  }

  public clear(): void {
    this.length = 0;
    this.head = 0;
  }

  /**
   * Produces contiguous linear slices directly compatible with uPlot [x, ...y]
   * Copies memory using fast TypedArray subarray/set operations.
   */
  public getLinearData(): [
    Float64Array,
    Float64Array,
    Float64Array,
    Float64Array,
    Float64Array,
  ] {
    const n = this.length;
    if (n === 0) {
      return [
        this.exportTime.subarray(0, 0),
        this.exportRate.subarray(0, 0),
        this.exportThroughput.subarray(0, 0),
        this.exportJitter.subarray(0, 0),
        this.exportCrc.subarray(0, 0),
      ];
    }

    if (this.head === 0) {
      // Direct fast copy
      this.exportTime.set(this.timestamps.subarray(0, n));
      this.exportRate.set(this.packetRate.subarray(0, n));
      this.exportThroughput.set(this.throughputKbps.subarray(0, n));
      this.exportJitter.set(this.jitterMs.subarray(0, n));
      this.exportCrc.set(this.crcErrors.subarray(0, n));
    } else {
      // Circular split wrap-around copy
      const part1 = this.capacity - this.head;
      const part2 = n - part1;

      // Part 1: [head .. capacity]
      this.exportTime.set(this.timestamps.subarray(this.head, this.capacity), 0);
      this.exportRate.set(this.packetRate.subarray(this.head, this.capacity), 0);
      this.exportThroughput.set(this.throughputKbps.subarray(this.head, this.capacity), 0);
      this.exportJitter.set(this.jitterMs.subarray(this.head, this.capacity), 0);
      this.exportCrc.set(this.crcErrors.subarray(this.head, this.capacity), 0);

      // Part 2: [0 .. part2]
      if (part2 > 0) {
        this.exportTime.set(this.timestamps.subarray(0, part2), part1);
        this.exportRate.set(this.packetRate.subarray(0, part2), part1);
        this.exportThroughput.set(this.throughputKbps.subarray(0, part2), part1);
        this.exportJitter.set(this.jitterMs.subarray(0, part2), part1);
        this.exportCrc.set(this.crcErrors.subarray(0, part2), part1);
      }
    }

    return [
      this.exportTime.subarray(0, n),
      this.exportRate.subarray(0, n),
      this.exportThroughput.subarray(0, n),
      this.exportJitter.subarray(0, n),
      this.exportCrc.subarray(0, n),
    ];
  }
}
