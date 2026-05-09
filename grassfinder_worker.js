import init, { ScanPlan } from "./wasm/pkg/grassfinder_wasm.js";

let ready = init();

const SEED_POST_1_8 = 0;
const SEED_PRE_1_8 = 1;
const SEED_B1_6_TB3 = 2;

function seedModeForVersion(version) {
  if (version === "post1_12") return SEED_POST_1_8;
  if (version === "b1_6_tb3") return SEED_B1_6_TB3;
  return SEED_PRE_1_8;
}

self.onmessage = async (e) => {
  await ready;

  const {
    jobId,
    x0, x1,
    z0, z1,
    y0, y1,
    version,
    relDx, relDy, relDz,
    relPacked, relMask, relDrip,
    maxMatches,
    post1_12_anyY,
    mode,
    tol,
    maxScore
  } = e.data;

  const post1_12 = (version === "post1_12");
  const anyY = !!post1_12_anyY || post1_12;
  const seedMode = seedModeForVersion(version);
  const plan = new ScanPlan(relDx, relDy, relDz, relPacked, relMask, relDrip, seedMode);

  const xCount = (x1 - x0 + 1);
  const zCount = (z1 - z0 + 1);
  const yCount = anyY ? 1 : (y1 - y0 + 1);
  const total = xCount * zCount * yCount;

  let done = 0;
  const matches = [];

  // Chunk by Z so we can emit progress periodically (similar feel to the JS worker).
  // Strict Y-dependent scans use a WASM Y-prefilter, so their real hot-loop cost is
  // closer to X/Z cells than X/Y/Z cells. Larger chunks avoid thousands of tiny
  // worker/WASM calls on large pre-1.8 searches while keeping progress responsive.
  const emitEvery = mode === "scored" ? 500000 : 2000000;
  const chunkYCost = (!anyY && mode !== "scored") ? 1 : yCount;
  const zChunk = Math.max(1, Math.floor(emitEvery / (xCount * chunkYCost)));

  try {
    for (let zs = z0; zs <= z1; zs += zChunk) {
      const ze = Math.min(z1, zs + zChunk - 1);

      const remaining = Math.max(0, (maxMatches | 0) - matches.length);
      if (remaining === 0) {
        self.postMessage({ jobId, type: "done", done, total, matches, hitCap: true });
        return;
      }

      if (mode === "scored") {
        const arr = plan.scan_scored_box(
          x0, x1, y0, y1, zs, ze,
          remaining,
          tol | 0,
          maxScore | 0
        );

        for (let i = 0; i < arr.length; i += 4) {
          matches.push({ x: arr[i], y: arr[i + 1], z: arr[i + 2], score: arr[i + 3] });
        }
      } else {
        const arr = plan.scan_strict_box(
          x0, x1, y0, y1, zs, ze,
          remaining
        );

        for (let i = 0; i < arr.length; i += 3) {
          matches.push({ x: arr[i], y: arr[i + 1], z: arr[i + 2] });
        }
      }

      done += xCount * (ze - zs + 1) * yCount;

      if (matches.length >= (maxMatches | 0)) {
        self.postMessage({ jobId, type: "done", done, total, matches, hitCap: true });
        return;
      }

      self.postMessage({ jobId, type: "progress", done, total, matchesCount: matches.length });
    }
  } finally {
    plan.free();
  }

  self.postMessage({ jobId, type: "done", done, total, matches, hitCap: false });
};
