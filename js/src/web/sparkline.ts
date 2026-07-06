// Live sparkline of per-frame decode times against the frame budget.
// Palette per docs' dataviz method: series slot-1 dark blue validated ≥3:1
// on the #101216 chart surface; chrome uses muted/baseline ink, not series
// color.
const SERIES = "#3987e5";
const BUDGET_LINE = "#898781";
const BASELINE = "#383835";
const LABEL = "#898781";

const CAPACITY = 240;

export class Sparkline {
  private readonly canvas: HTMLCanvasElement;
  private readonly context: CanvasRenderingContext2D;
  private readonly samples: number[] = [];
  private budgetMs = 0;
  private lastDrawMs = -Infinity;

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas;
    const context = canvas.getContext("2d");
    if (context === null) {
      throw new Error("canvas 2d context unavailable");
    }
    this.context = context;
  }

  reset(): void {
    this.samples.length = 0;
    this.budgetMs = 0;
    this.draw();
  }

  push(sampleMs: number, budgetMs: number): void {
    this.samples.push(sampleMs);
    if (this.samples.length > CAPACITY) {
      this.samples.shift();
    }
    this.budgetMs = budgetMs;
    // Redrawing the polyline per decoded frame steals ~3ms/frame from the
    // decode wave on 4-core devices; 4Hz reads the same.
    const now = performance.now();
    if (now - this.lastDrawMs >= 250) {
      this.lastDrawMs = now;
      this.draw();
    }
  }

  private draw(): void {
    const dpr = window.devicePixelRatio;
    const cssWidth = this.canvas.clientWidth;
    const cssHeight = this.canvas.clientHeight;
    const width = Math.round(cssWidth * dpr);
    const height = Math.round(cssHeight * dpr);
    if (width === 0 || height === 0) {
      return;
    }
    if (this.canvas.width !== width || this.canvas.height !== height) {
      this.canvas.width = width;
      this.canvas.height = height;
    }
    const ctx = this.context;
    ctx.clearRect(0, 0, width, height);

    const pad = 2 * dpr;
    const plotHeight = height - 2 * pad;
    const yMax = Math.max(this.budgetMs * 1.5, ...this.samples, 1);
    const y = (ms: number): number => pad + plotHeight * (1 - ms / yMax);
    const x = (index: number): number => (width * index) / (CAPACITY - 1);

    ctx.lineWidth = 1;
    ctx.strokeStyle = BASELINE;
    ctx.beginPath();
    ctx.moveTo(0, height - 0.5);
    ctx.lineTo(width, height - 0.5);
    ctx.stroke();

    if (this.budgetMs > 0) {
      const budgetY = y(this.budgetMs);
      ctx.strokeStyle = BUDGET_LINE;
      ctx.setLineDash([3 * dpr, 3 * dpr]);
      ctx.beginPath();
      ctx.moveTo(0, budgetY);
      ctx.lineTo(width, budgetY);
      ctx.stroke();
      ctx.setLineDash([]);

      ctx.fillStyle = LABEL;
      ctx.font = `${10 * dpr}px system-ui, sans-serif`;
      ctx.textAlign = "right";
      ctx.textBaseline = "bottom";
      ctx.fillText(
        `budget ${this.budgetMs.toFixed(1)} ms`,
        width - 4 * dpr,
        budgetY - 2 * dpr,
      );
    }

    if (this.samples.length >= 2) {
      ctx.strokeStyle = SERIES;
      ctx.lineWidth = 1.5 * dpr;
      ctx.lineJoin = "round";
      ctx.beginPath();
      for (let i = 0; i < this.samples.length; i += 1) {
        const px = x(i);
        const py = y(this.samples[i]);
        if (i === 0) {
          ctx.moveTo(px, py);
        } else {
          ctx.lineTo(px, py);
        }
      }
      ctx.stroke();
    }
  }
}
