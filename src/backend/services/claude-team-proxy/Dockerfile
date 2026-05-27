## Multi-stage build for claude-team-proxy (issue #458).
##
## Two stages:
##   1. deps    — `npm ci --omit=dev`. Produces /app/node_modules.
##   2. runtime — runtime base + Chromium sys deps + browser binary +
##                source code. This is what ships.
##
## Why multi-stage: keeps the runtime image free of npm cache, dev-only
## tooling, intermediate apt artefacts. Build context churn doesn't
## bust the runtime layer cache as long as package.json is unchanged.
##
## Image size budget: ~600-700 MB (Chromium dominates).

# ─── Stage 1: deps ────────────────────────────────────────────────────
FROM node:22-slim AS deps

WORKDIR /app

# Copy lockfile + manifest first so this layer caches independently of
# source code changes. Any edit to .js files invalidates the runtime
# stage but leaves this layer reused, saving ~30s rebuilds.
COPY package.json package-lock.json ./

# `npm ci` is the production-deploy form of `npm install`:
#   - fails if package.json and package-lock.json disagree
#   - does not write to package-lock.json
#   - faster than `install` for cold installs
# `--omit=dev` skips devDependencies (we don't have any yet, but if we
# add e.g. a test runner later it stays out of the image).
RUN npm ci --omit=dev --no-audit --no-fund

# ─── Stage 2: runtime ─────────────────────────────────────────────────
FROM node:22-slim AS runtime

WORKDIR /app

# Bring in resolved node_modules from stage 1 (Playwright npm package
# included). Source code copied separately below so it can be changed
# without busting the deps layer.
COPY --from=deps /app/node_modules ./node_modules
COPY package.json ./

# Install Chromium browser + the apt packages it needs to run on
# Debian. `--with-deps` runs `apt-get install` for the canonical list
# of libraries (libnss3, libxkbcommon0, libgbm1, libcups2, libatk-…
# and friends). Doing it via playwright avoids hardcoding the list
# in our Dockerfile and drifting from upstream.
#
# PLAYWRIGHT_BROWSERS_PATH pins the binary location to a stable place
# under /app instead of the default ~/.cache/ms-playwright. Means the
# binary's path doesn't depend on the runtime user's HOME, and `chown`
# below covers it.
ENV PLAYWRIGHT_BROWSERS_PATH=/app/.playwright
RUN npx playwright install --with-deps chromium && \
    rm -rf /var/lib/apt/lists/*

# Source code last so editing src/* invalidates only this layer.
COPY src ./src

# Run as the pre-baked `node` user (UID/GID 1000) that ships with the
# official Node.js base image. Has its own HOME (/home/node) which is
# where some Chromium subsystems write small caches. Reusing the
# upstream user is more idiomatic than creating a parallel `app` user
# and avoids GID collisions (the upstream `node` group already owns
# 1000).
RUN chown -R node:node /app

USER node

# Defaults that make sense for k8s. Override via env at runtime.
ENV NODE_ENV=production \
    HEADLESS=true \
    PORT=3000

EXPOSE 3000

# `node` directly (not `npm start`) so PID 1 is the node process,
# which means SIGTERM from k8s reaches our shutdown handler directly.
# With `npm start` PID 1 is npm and signals don't always forward.
CMD ["node", "src/index.js"]
