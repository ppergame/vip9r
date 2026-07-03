// Procedural "night city flyover" renderer. One WebGL2 fragment shader,
// deterministic in (time, params): the same inputs always produce the same
// frame, so the preview loop and the decant encode pass share render().
//
// Scene model: an infinite grid of hashed building boxes marched with a 2D
// DDA over ground cells (exact ray/box hits, no SDF overstepping). All light
// is emissive — lit windows, street lamp pools, traffic dashes, roof beacons,
// sky glow — plus distance fog. Anti-aliasing is 2x2 supersampling with
// analytic (distance-derived) filter widths instead of fwidth, which keeps
// shading defined under divergent control flow.

export type SceneParams = {
  seed: number;
  speed: number;
  altitude: number;
  pitch: number;
  fov: number;
  sway: number;
  density: number;
  towers: number;
  lit: number;
  warmth: number;
  fog: number;
  glow: number;
  streets: number;
  traffic: number;
  neon: number;
  grain: number;
  exposure: number;
};

export const PARAM_KEYS = [
  "seed",
  "speed",
  "altitude",
  "pitch",
  "fov",
  "sway",
  "density",
  "towers",
  "lit",
  "warmth",
  "fog",
  "glow",
  "streets",
  "traffic",
  "neon",
  "grain",
  "exposure",
] as const satisfies readonly (keyof SceneParams)[];

const VERTEX_SHADER = `#version 300 es
void main() {
  vec2 corner = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
  gl_Position = vec4(corner * 2.0 - 1.0, 0.0, 1.0);
}
`;

const FRAGMENT_SHADER = `#version 300 es
precision highp float;
precision highp int;

uniform vec2 u_res;
uniform float u_time;
uniform float u_frame;
uniform float u_samples;
${PARAM_KEYS.map((key) => `uniform float u_${key};`).join("\n")}

out vec4 outColor;

const float CELL = 26.0;
const float MAXDIST = 4200.0;
const int MAXSTEP = 200;

// --- hashing ------------------------------------------------------------

uint pcg(uint v) {
  uint state = v * 747796405u + 2891336453u;
  uint word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
  return (word >> 22u) ^ word;
}

float uf(uint h) {
  return float(h) * (1.0 / 4294967296.0);
}

// Single-round cell hash: this sits in the DDA inner loop, so it trades hash
// quality for ALU count (one pcg round over a multiply-mixed key).
uint hashCell(ivec2 c, uint salt) {
  uvec2 p = uvec2(c + ivec2(0x40000000));
  return pcg(p.x * 0x9e3779b9u ^ p.y * 0x85ebca6bu ^ (salt + uint(u_seed) * 0x1000193u));
}

// Bilinear value noise over cell coordinates; drives downtown clustering.
float vnoise(vec2 p, uint salt) {
  vec2 i = floor(p);
  vec2 f = p - i;
  vec2 u = f * f * (3.0 - 2.0 * f);
  ivec2 ii = ivec2(i);
  float a = uf(hashCell(ii, salt));
  float b = uf(hashCell(ii + ivec2(1, 0), salt));
  float c = uf(hashCell(ii + ivec2(0, 1), salt));
  float d = uf(hashCell(ii + ivec2(1, 1), salt));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float byteF(uint word, uint shift) {
  return float((word >> shift) & 0xffu) * (1.0 / 255.0);
}

// --- grid space ------------------------------------------------------------
// The city grid is rotated by a seed-derived angle relative to the flight
// path (which runs along world +z), so the camera never stares down an
// infinite one-point-perspective street corridor. March and shading run in
// grid space; only the flight corridor needs to map back to world x.

vec2 g_cs; // (cos, sin) of the grid angle, set once in main()

vec2 toGrid(vec2 w) {
  return vec2(g_cs.x * w.x - g_cs.y * w.y, g_cs.y * w.x + g_cs.x * w.y);
}

float worldXof(vec2 g) {
  return g_cs.x * g.x + g_cs.y * g.y;
}

// --- city model ----------------------------------------------------------

struct Bld {
  bool present;
  vec3 lo;
  vec3 hi;
  uint id;
};

Bld getBld(ivec2 c) {
  Bld b;
  b.present = false;

  // Apartment-complex districts ("danji"): outside downtown, some 6x6-cell
  // districts become rows of identical slabs with a spacing row between —
  // uniform geometry per district, per-building light patterns.
  ivec2 dc = ivec2(floor(vec2(c) / 6.0));
  uint dH = hashCell(dc, 91u);
  if (uf(dH) < 0.22) {
    float dDowntown = smoothstep(0.45, 0.85, vnoise(vec2(dc) * (6.0 / 18.0), 3u));
    if (dDowntown < 0.45) {
      uint d1 = pcg(dH);
      bool alongX = (d1 & 1u) == 0u;
      if ((alongX ? (c.y & 1) : (c.x & 1)) == 1) {
        return b; // green space between slab rows
      }
      float height = mix(34.0, 58.0, byteF(d1, 8u)) + 4.0 * byteF(hashCell(c, 8u), 0u);
      vec2 center = (vec2(c) + 0.5) * CELL;
      if (abs(worldXof(center)) < 90.0) {
        height = min(height, max(u_altitude - 45.0, 25.0));
      }
      vec2 ext = alongX ? vec2(10.5, 4.5) : vec2(4.5, 10.5);
      b.present = true;
      b.id = hashCell(c, 7u);
      b.lo = vec3(center.x - ext.x, 0.0, center.y - ext.y);
      b.hi = vec3(center.x + ext.x, height, center.y + ext.y);
      return b;
    }
  }

  // 2x2 super-block layer: some blocks are parks/plazas (empty), some are one
  // fat low slab (mall, depot). Every cell of a block derives the same answer.
  ivec2 blockC = c >> 1;
  uint blockH = hashCell(blockC, 55u);
  float blockMode = uf(blockH);
  if (blockMode < 0.10) {
    return b; // park / plaza: a hole in the skyline
  }
  if (blockMode < 0.26) {
    uint s1 = pcg(blockH);
    vec2 center = (vec2(blockC) * 2.0 + 1.0) * CELL;
    float height = mix(8.0, 26.0, byteF(s1, 0u));
    if (abs(worldXof(center)) < 90.0) {
      height = min(height, max(u_altitude - 45.0, 25.0));
    }
    vec2 ext = vec2(mix(15.0, 21.0, byteF(s1, 8u)), mix(15.0, 21.0, byteF(s1, 16u)));
    center += (vec2(byteF(s1, 24u), byteF(pcg(s1), 0u)) - 0.5) * 2.0 * (vec2(24.0) - ext);
    b.present = true;
    b.id = blockH;
    b.lo = vec3(center.x - ext.x, 0.0, center.y - ext.y);
    b.hi = vec3(center.x + ext.x, height, center.y + ext.y);
    return b;
  }

  uint id = hashCell(c, 7u);
  if (uf(id) > u_density) {
    return b;
  }
  float downtown = smoothstep(0.45, 0.85, vnoise(vec2(c) / 18.0, 3u));
  // All per-building attributes come from byte slices of two follow-up words.
  uint h1 = pcg(id);
  uint h2 = pcg(h1);
  float r1 = byteF(h1, 0u);
  float base = mix(10.0, 42.0, r1 * r1);
  float tall = step(1.0 - u_towers * (0.10 + 0.90 * downtown), byteF(h1, 8u));
  float height = base + downtown * 25.0 + tall * mix(50.0, 130.0, byteF(h1, 16u));
  vec2 center = (vec2(c) + 0.5) * CELL;
  // Flight corridor: never grow into the camera path.
  if (abs(worldXof(center)) < 90.0) {
    height = min(height, max(u_altitude - 45.0, 25.0));
  }
  vec2 ext = vec2(mix(5.5, 9.5, byteF(h1, 24u)), mix(5.5, 9.5, byteF(h2, 0u)));
  vec2 jitterMax = max(vec2(CELL * 0.5 - 4.0) - ext, vec2(0.0));
  center += (vec2(byteF(h2, 8u), byteF(h2, 16u)) - 0.5) * 2.0 * jitterMax;
  b.present = true;
  b.id = id;
  b.lo = vec3(center.x - ext.x, 0.0, center.y - ext.y);
  b.hi = vec3(center.x + ext.x, height, center.y + ext.y);
  return b;
}

// --- marching ------------------------------------------------------------

float safeInv(float v) {
  return 1.0 / (v >= 0.0 ? max(v, 1e-8) : min(v, -1e-8));
}

struct Hit {
  int kind; // 0 sky, 1 building, 2 ground
  float t;
  vec3 n;
  uint id;
  float height;
  vec2 center;
};

Hit march(vec3 ro, vec3 rd) {
  Hit h;
  h.kind = 0;
  h.t = MAXDIST;

  float tGround = rd.y < -1e-4 ? -ro.y / rd.y : MAXDIST;
  float tLimit = min(MAXDIST, tGround);
  vec3 inv = vec3(safeInv(rd.x), safeInv(rd.y), safeInv(rd.z));

  ivec2 c = ivec2(floor(ro.xz / CELL));
  ivec2 stp = ivec2(rd.x >= 0.0 ? 1 : -1, rd.z >= 0.0 ? 1 : -1);
  vec2 deltaT = abs(vec2(CELL * inv.x, CELL * inv.z));
  vec2 nextBound = (vec2(c) + step(vec2(0.0), rd.xz)) * CELL;
  vec2 tMax = (nextBound - ro.xz) * inv.xz;

  for (int i = 0; i < MAXSTEP; i += 1) {
    Bld b = getBld(c);
    if (b.present) {
      vec3 t0 = (b.lo - ro) * inv;
      vec3 t1 = (b.hi - ro) * inv;
      vec3 tsmall = min(t0, t1);
      vec3 tbig = max(t0, t1);
      float tn = max(max(tsmall.x, tsmall.y), tsmall.z);
      float tf = min(min(tbig.x, tbig.y), tbig.z);
      if (tn < tf && tn > 0.0 && tn < tLimit) {
        h.kind = 1;
        h.t = tn;
        h.n = -sign(rd) * step(vec3(tn), tsmall);
        h.id = b.id;
        h.height = b.hi.y;
        h.center = (b.lo.xz + b.hi.xz) * 0.5;
        return h;
      }
    }
    float tNext = min(tMax.x, tMax.y);
    if (tMax.x < tMax.y) {
      c.x += stp.x;
      tMax.x += deltaT.x;
    } else {
      c.y += stp.y;
      tMax.y += deltaT.y;
    }
    if (tNext > tLimit) {
      break;
    }
    // Climbing rays: nothing to hit once above the tallest possible building.
    if (rd.y > 0.0 && ro.y + tNext * rd.y > 245.0) {
      break;
    }
  }

  if (tGround < MAXDIST) {
    h.kind = 2;
    h.t = tGround;
    h.n = vec3(0.0, 1.0, 0.0);
  }
  return h;
}

// --- shading -------------------------------------------------------------

const vec3 WARM = vec3(1.0, 0.80, 0.52);
const vec3 COOL = vec3(0.58, 0.74, 1.0);
const vec3 SODIUM = vec3(1.0, 0.58, 0.22);

// Weighted toward red/amber/warm-white, matching Korean signage.
const vec3 SIGN_PALETTE[6] = vec3[6](
  vec3(1.00, 0.07, 0.12), vec3(1.00, 0.55, 0.10), vec3(1.00, 0.90, 0.70),
  vec3(0.10, 0.80, 1.00), vec3(1.00, 0.07, 0.12), vec3(0.90, 0.15, 0.60)
);

vec3 shadeBuilding(vec3 p, Hit h, vec3 rd, float pixAng) {
  if (h.n.y > 0.5) {
    // Roof: near-black, pulsing red beacon on mid+ buildings.
    vec3 col = vec3(0.006, 0.007, 0.010);
    if (h.height > 45.0 && uf(pcg(h.id ^ 77u)) < 0.45) {
      float d2 = dot(p.xz - h.center, p.xz - h.center);
      float rate = mix(1.5, 3.0, uf(pcg(h.id ^ 141u)));
      float pulse = 0.5 + 0.5 * sin(u_time * rate + float(h.id & 255u) * 0.13);
      col += vec3(1.0, 0.04, 0.04) * pulse * exp(-d2 / 1.2) * 4.0;
    }
    // Teal-lit roofs on some of the slab stock (seoul3 reference).
    if (h.height > 28.0 && h.height < 75.0) {
      uint rH = pcg(h.id ^ 0x7ea1u);
      if (uf(rH) < 0.15 * u_neon) {
        col += vec3(0.06, 0.45, 0.55) * mix(0.08, 0.28, byteF(rH, 8u));
      }
    }
    // Steady red church crosses sprinkled over the low-rise fabric.
    if (h.height < 40.0) {
      uint cH = pcg(h.id ^ 0xc705u);
      if (uf(cH) < 0.15 * u_neon) {
        vec2 offs = (vec2(byteF(cH, 8u), byteF(cH, 16u)) - 0.5) * 6.0;
        vec2 d = p.xz - (h.center + offs);
        col += vec3(1.0, 0.02, 0.05) * exp(-dot(d, d) / 0.35) * 5.0;
      }
    }
    return col;
  }

  vec3 col = vec3(0.006, 0.007, 0.011);

  // Window lattice in world-aligned facade coordinates.
  vec2 fc = abs(h.n.x) > 0.5 ? vec2(p.z, p.y) : vec2(p.x, p.y);
  vec2 pitchWH = vec2(3.0, 3.4);
  fc.x += uf(pcg(h.id ^ 101u)) * 7.0;
  vec2 wc = fc / pitchWH;
  vec2 wi = floor(wc);
  vec2 wf = wc - wi;

  // Analytic filter width: projected pixel footprint in window-lattice units.
  float cosI = max(abs(dot(rd, h.n)), 0.2);
  vec2 fw = vec2(h.t * pixAng / cosI) / pitchWH;
  vec2 aa = clamp(fw, vec2(0.02), vec2(0.6));
  vec2 lo = vec2(0.20, 0.30);
  vec2 hi = vec2(0.80, 0.85);
  vec2 edges = smoothstep(lo - aa, lo + aa, wf) * (1.0 - smoothstep(hi - aa, hi + aa, wf));
  float mask = edges.x * edges.y;

  uvec2 wid = uvec2(ivec2(wi) + ivec2(0x4000));
  float bias = mix(0.4, 1.6, uf(pcg(h.id ^ 0x9e37u)));
  float litP = clamp(u_lit * bias, 0.0, 0.97);
  float perWin = uf(pcg(wid.x + pcg(wid.y + h.id)));
  float bright = mix(0.25, 1.0, uf(pcg(wid.y + pcg(wid.x + (h.id ^ 5u)))));

  // Lighting archetype: i.i.d. windows read as TV static, so correlate them.
  // Offices light whole floors, hotels light stairwell/elevator columns,
  // residential keeps the sparse random sprinkle.
  float archR = uf(pcg(h.id ^ 0xa1ceu));
  float officeP = mix(0.25, 0.65, smoothstep(30.0, 80.0, h.height));
  float lit;
  vec3 wcol;
  if (archR < officeP) {
    float floorOn = step(uf(pcg(wid.y ^ (h.id * 3u))), litP * 1.15);
    lit = floorOn * step(0.15, perWin);
    bright = mix(0.55, 0.85, perWin);
    // Fluorescent office floors: blue-white to green-white per building.
    wcol = mix(vec3(0.65, 0.82, 1.0), vec3(0.70, 0.95, 0.78), uf(pcg(h.id ^ 0x0f1u)));
  } else if (archR < officeP + 0.15) {
    float colOn = step(uf(pcg(wid.x ^ (h.id * 5u))), litP * 0.85);
    lit = max(colOn * step(0.25, perWin), step(perWin, litP * 0.25));
    wcol = WARM;
  } else {
    lit = step(perWin, litP * 0.9);
    float temper = uf(pcg(wid.x * 3u + pcg(wid.y + (h.id ^ 9u))));
    wcol = mix(COOL, WARM, step(temper, u_warmth));
  }

  // Ground floor reads as storefronts: brighter, warmer, more often lit.
  if (wi.y < 0.5) {
    lit = step(perWin, min(litP * 2.0, 0.95));
    bright *= 1.5;
    wcol = mix(wcol, WARM, 0.5);
  }
  // Parapet: no windows peeking over the roofline.
  if (p.y > h.height - 2.2) {
    mask = 0.0;
  }

  // Stacked commercial signage over floors 1-3: saturated panels in 6 m
  // segments, occasionally pulsing. Replaces the windows it covers.
  vec3 signEmit = vec3(0.0);
  float signMask = 0.0;
  if (u_neon > 0.001 && wi.y >= 0.5 && wi.y < 3.5 && p.y < h.height - 3.0) {
    float signBias = h.height > 70.0 ? 0.15 : 0.35;
    if (uf(pcg(h.id ^ 0x516eu)) < signBias) {
      float sc = floor(fc.x / 6.0);
      uint sH = pcg(uint(int(sc) + 0x8000) + pcg(uint(int(wi.y)) * 19u + h.id));
      if (uf(sH) < 0.45) {
        vec2 sf = vec2(fract(fc.x / 6.0), wf.y);
        vec2 saa = clamp(vec2(fw.x * 0.5, fw.y), vec2(0.01), vec2(0.45));
        vec2 pm = smoothstep(vec2(0.10, 0.18) - saa, vec2(0.10, 0.18) + saa, sf)
                * (1.0 - smoothstep(vec2(0.90, 0.82) - saa, vec2(0.90, 0.82) + saa, sf));
        signMask = pm.x * pm.y;
        vec3 hue = SIGN_PALETTE[int((sH >> 8u) % 6u)];
        float pulse = 1.0;
        if (((sH >> 16u) & 3u) == 0u) {
          pulse = 0.75 + 0.25 * sin(u_time * mix(0.8, 2.5, byteF(sH, 20u)) + float(sH & 63u));
        }
        signEmit = hue * (2.0 * u_neon * pulse) * signMask;
      }
    }
  }

  // Windows sit well below street/signage intensity — in the reference
  // aerials the yellow lives in the streets, not the building mass.
  vec3 winEmit = wcol * (lit * bright) * mask * (1.0 - signMask) * 1.4;
  // Beyond lattice resolution, converge to the facade's average emission.
  vec3 avgEmit = mix(COOL, WARM, u_warmth) * vec3(1.0, 0.85, 0.70) * (litP * 0.055) * 2.5;
  float lod = smoothstep(0.6, 2.2, max(fw.x, fw.y));
  col += mix(winEmit, avgEmit, lod) + signEmit;

  // Amber crown signage band near the roofline of mid-rise slabs.
  if (u_neon > 0.001 && h.height > 30.0 && h.height < 65.0 && p.y > h.height - 4.6 && p.y < h.height - 2.4) {
    uint kH = pcg(h.id ^ 0x9317u);
    if (uf(kH) < 0.35) {
      float sMid = fract(fc.x / 22.0);
      float bandMask = smoothstep(0.15, 0.25, sMid) * (1.0 - smoothstep(0.75, 0.85, sMid));
      col += vec3(1.0, 0.62, 0.18) * (1.3 * u_neon * bandMask);
    }
  }

  // Street-level uplight on the facade base.
  col += SODIUM * (u_streets * 0.05 * exp(-p.y / 10.0));
  return col;
}

vec3 shadeGround(vec3 p, vec3 rd, float t, float pixAng) {
  float fwWorld = t * pixAng / max(abs(rd.y), 0.1);

  vec2 f = fract(p.xz / CELL);
  vec2 db = (0.5 - abs(f - 0.5)) * CELL;
  float streetX = 1.0 - smoothstep(3.5, 4.5, db.y); // runs along x, near a z boundary
  float streetZ = 1.0 - smoothstep(3.5, 4.5, db.x);
  float street = max(streetX, streetZ);

  vec3 col = mix(vec3(0.004, 0.005, 0.007), vec3(0.011, 0.012, 0.015), street);

  // Sodium lamp pools every 24 m along each street, alternating sides.
  float lodG = smoothstep(4.0, 14.0, fwWorld);
  vec2 boundary = floor(p.xz / CELL + 0.5) * CELL;
  float pools = 0.0;
  {
    float s = floor(p.x / 24.0 + 0.5) * 24.0;
    float side = mod(s / 24.0, 2.0) < 1.0 ? 3.0 : -3.0;
    vec2 lamp = vec2(s, boundary.y + side);
    float d2 = dot(p.xz - lamp, p.xz - lamp);
    pools += exp(-d2 / 20.0) * streetX;
  }
  {
    float s = floor(p.z / 24.0 + 0.5) * 24.0;
    float side = mod(s / 24.0, 2.0) < 1.0 ? 3.0 : -3.0;
    vec2 lamp = vec2(boundary.x + side, s);
    float d2 = dot(p.xz - lamp, p.xz - lamp);
    pools += exp(-d2 / 20.0) * streetZ;
  }
  // Unresolvable lamps flatten into a uniform street glow.
  col += SODIUM * (u_streets * 0.55 * mix(pools, 0.16 * street, lodG));

  // Signage spill: each street segment picks up a random neon tint.
  if (u_neon > 0.001) {
    uint gx = hashCell(ivec2(int(floor(p.x / CELL)), int(boundary.y / CELL) * 3 + 1000), 77u);
    uint gz = hashCell(ivec2(int(boundary.x / CELL) * 3 + 2000, int(floor(p.z / CELL))), 78u);
    vec3 spill = SIGN_PALETTE[int(gx % 6u)] * (step(uf(gx), 0.35) * streetX)
               + SIGN_PALETTE[int(gz % 6u)] * (step(uf(gz), 0.35) * streetZ);
    col += spill * (u_neon * 0.06 * (1.0 - lodG));
  }

  // Traffic: headlight/taillight dashes in two lanes per street.
  if (u_traffic > 0.001) {
    float lodT = 1.0 - smoothstep(2.0, 8.0, fwWorld);
    vec3 dashes = vec3(0.0);
    for (int axis = 0; axis < 2; axis += 1) {
      float along = axis == 0 ? p.x : p.z;
      float across = axis == 0 ? p.z - boundary.y : p.x - boundary.x;
      float onStreet = axis == 0 ? streetX : streetZ;
      uint streetSeed = pcg(uint(int((axis == 0 ? boundary.y : boundary.x) / CELL) + 0x40000) ^ (uint(axis) * 733u));
      for (int lane = 0; lane < 2; lane += 1) {
        float dir = lane == 0 ? 1.0 : -1.0;
        float laneMask = exp(-pow((across - dir * 1.7) / 0.9, 2.0)) * onStreet;
        uint laneSeed = pcg(streetSeed ^ (uint(lane) + 41u));
        float spacing = mix(34.0, 60.0, uf(laneSeed));
        float u = (along - dir * mix(11.0, 17.0, uf(pcg(laneSeed))) * u_time) / spacing + uf(laneSeed ^ 3u);
        float du = fract(u);
        float carLen = 4.5 / spacing;
        float aaU = clamp(fwWorld / spacing, 0.02, 0.5);
        float dash = smoothstep(-aaU, aaU, du) * (1.0 - smoothstep(carLen - aaU, carLen + aaU, du));
        vec3 carCol = dir > 0.0 ? vec3(1.0, 0.93, 0.80) : vec3(1.0, 0.10, 0.05);
        dashes += carCol * dash * laneMask;
      }
    }
    col += dashes * (u_traffic * 2.5 * lodT);
  }
  return col;
}

vec3 sky(vec3 rd) {
  float horizon = pow(clamp(1.0 - abs(rd.y), 0.0, 1.0), 14.0);
  vec3 glowc = vec3(0.88, 0.42, 0.24) * u_glow;
  return vec3(0.004, 0.005, 0.010) + glowc * horizon * 0.15;
}

vec3 shadeRay(vec3 ro, vec3 rd, float pixAng) {
  Hit h = march(ro, rd);
  if (h.kind == 0) {
    return sky(rd);
  }
  vec3 p = ro + rd * h.t;
  vec3 col = h.kind == 1 ? shadeBuilding(p, h, rd, pixAng) : shadeGround(p, rd, h.t, pixAng);
  // Ground haze: thick in the street canyons, thin around tower tops.
  float heightFade = 0.30 + 0.70 * exp(-max(p.y, 0.0) / 130.0);
  float fogAmt = 1.0 - exp(-pow(h.t * u_fog * heightFade / 900.0, 2.0));
  vec3 fogCol = vec3(0.30, 0.16, 0.11) * (u_glow * 0.16) + vec3(0.006, 0.008, 0.014);
  return mix(col, fogCol, fogAmt);
}

// --- camera --------------------------------------------------------------

// Two incommensurate frequencies per axis plus a slow drift, phases from the
// seed: the sway never visibly repeats within a clip. Lateral amplitude stays
// under the corridor half-width (90) that getBld carves.
vec3 camPath(float t) {
  uint sd = uint(u_seed);
  float p1 = 6.2832 * uf(pcg(sd + 11u));
  float p2 = 6.2832 * uf(pcg(sd + 12u));
  float p3 = 6.2832 * uf(pcg(sd + 13u));
  return vec3(
    u_sway * (28.0 * sin(0.081 * t + p1) + 17.0 * sin(0.0501 * t + p2)),
    u_altitude + u_sway * (6.0 * sin(0.043 * t + p3) + 10.0 * sin(0.0071 * t + p2)),
    u_speed * t
  );
}

mat3 camBasis(float t, out vec3 ro) {
  ro = camPath(t);
  vec3 behind = camPath(t - 2.0);
  vec3 ahead = camPath(t + 2.0);
  vec3 f = normalize(ahead - ro);
  float yaw = atan(f.x, f.z);
  float pitch = asin(clamp(f.y, -1.0, 1.0)) - radians(u_pitch);
  vec3 fwd = vec3(sin(yaw) * cos(pitch), sin(pitch), cos(yaw) * cos(pitch));
  vec3 right0 = normalize(cross(vec3(0.0, 1.0, 0.0), fwd));
  vec3 up0 = cross(fwd, right0);
  // Bank into the turn: roll follows lateral acceleration (second difference).
  float latAcc = (ahead.x - 2.0 * ro.x + behind.x) * 0.25;
  float roll = clamp(-1.2 * latAcc, -0.4, 0.4);
  vec3 right = right0 * cos(roll) + up0 * sin(roll);
  vec3 up = -right0 * sin(roll) + up0 * cos(roll);
  return mat3(right, up, fwd);
}

// --- main ----------------------------------------------------------------

void main() {
  float gridAngle = 6.2832 * uf(pcg(uint(u_seed) + 0x51u));
  g_cs = vec2(cos(gridAngle), sin(gridAngle));

  vec3 ro;
  mat3 basis = camBasis(u_time, ro);
  vec2 rog = toGrid(ro.xz);
  vec3 roG = vec3(rog.x, ro.y, rog.y);
  float tanF = tan(radians(u_fov) * 0.5);
  float pixAng = 2.0 * tanF / u_res.y;

  const vec2 OFFSETS[4] = vec2[4](
    vec2(-0.25, -0.25), vec2(0.25, -0.25), vec2(-0.25, 0.25), vec2(0.25, 0.25)
  );
  vec3 acc = vec3(0.0);
  for (int s = 0; s < 4; s += 1) {
    if (float(s) >= u_samples) {
      break;
    }
    vec2 frag = gl_FragCoord.xy + OFFSETS[s];
    vec2 uv = (2.0 * frag - u_res) / u_res.y;
    vec3 rd = basis * normalize(vec3(uv * tanF, 1.0));
    vec2 rdg = toGrid(rd.xz);
    acc += shadeRay(roG, vec3(rdg.x, rd.y, rdg.y), pixAng);
  }

  vec3 col = acc / u_samples;
  col = vec3(1.0) - exp(-col * u_exposure);
  col = pow(col, vec3(1.0 / 2.2));
  // Grain doubles as dither against banding in the dark gradients.
  float gr = uf(pcg(uint(gl_FragCoord.x) + pcg(uint(gl_FragCoord.y) * 7u + pcg(uint(u_frame))))) - 0.5;
  col += gr * u_grain;
  outColor = vec4(clamp(col, 0.0, 1.0), 1.0);
}
`;

export type Scene = {
  info: WebGlInfo;
  render(time: number, params: SceneParams, samples?: number): void;
};

export type WebGlInfo = {
  renderer: string;
  vendor: string;
  unmaskedRenderer: string | null;
  unmaskedVendor: string | null;
  version: string;
  shadingLanguageVersion: string;
};

export function createScene(canvas: HTMLCanvasElement): Scene {
  const gl = canvas.getContext("webgl2", {
    // The decant path constructs VideoFrames from the canvas after render().
    preserveDrawingBuffer: true,
    antialias: false,
    powerPreference: "high-performance",
  });
  if (gl === null) {
    throw new Error("WebGL2 is unavailable");
  }

  const program = gl.createProgram();
  for (const [type, source] of [
    [gl.VERTEX_SHADER, VERTEX_SHADER],
    [gl.FRAGMENT_SHADER, FRAGMENT_SHADER],
  ] as const) {
    const shader = gl.createShader(type);
    if (shader === null) {
      throw new Error("shader allocation failed");
    }
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      throw new Error(`shader compile: ${gl.getShaderInfoLog(shader)}`);
    }
    gl.attachShader(program, shader);
  }
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    throw new Error(`program link: ${gl.getProgramInfoLog(program)}`);
  }
  gl.useProgram(program);

  const uniform = (name: string): WebGLUniformLocation | null =>
    gl.getUniformLocation(program, name);
  const resLoc = uniform("u_res");
  const timeLoc = uniform("u_time");
  const frameLoc = uniform("u_frame");
  const samplesLoc = uniform("u_samples");
  const paramLocs = PARAM_KEYS.map((key) => uniform(`u_${key}`));
  const debugInfo = gl.getExtension("WEBGL_debug_renderer_info");

  return {
    info: {
      renderer: gl.getParameter(gl.RENDERER) as string,
      vendor: gl.getParameter(gl.VENDOR) as string,
      unmaskedRenderer:
        debugInfo === null
          ? null
          : (gl.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL) as string),
      unmaskedVendor:
        debugInfo === null
          ? null
          : (gl.getParameter(debugInfo.UNMASKED_VENDOR_WEBGL) as string),
      version: gl.getParameter(gl.VERSION) as string,
      shadingLanguageVersion: gl.getParameter(
        gl.SHADING_LANGUAGE_VERSION,
      ) as string,
    },
    // samples: 2x2 supersampling rays per pixel (1 = fast preview, 4 = decant).
    render(time: number, params: SceneParams, samples = 1): void {
      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.uniform2f(resLoc, canvas.width, canvas.height);
      gl.uniform1f(timeLoc, time);
      gl.uniform1f(frameLoc, Math.round(time * 30));
      gl.uniform1f(samplesLoc, samples);
      for (const [index, key] of PARAM_KEYS.entries()) {
        gl.uniform1f(paramLocs[index], params[key]);
      }
      gl.drawArrays(gl.TRIANGLES, 0, 3);
    },
  };
}
