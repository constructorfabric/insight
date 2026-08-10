FROM node:24-bookworm-slim AS builder

WORKDIR /app

COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./

# Not corepack: node images past 24 no longer ship it, and its package collides
# with the yarn bins this image already holds.
RUN npm install -g "pnpm@$(node -p 'require("/app/package.json").packageManager.split("@")[1]')"

RUN pnpm install --frozen-lockfile --ignore-scripts

COPY . .

ARG VITE_SENTRY_DSN
ARG VITE_APP_RELEASE
# Secret contents are not part of the layer cache key; this is.
ARG SENTRY_UPLOAD

RUN --mount=type=secret,id=sentry_env \
    if [ -f /run/secrets/sentry_env ]; then \
      set -a; . /run/secrets/sentry_env; set +a; \
    fi; \
    pnpm run build

# Runtime base: keep this on a currently-maintained nginx line. The 1.27 tag
# stopped getting Alpine package refreshes, so the published image accumulated
# fixable OS CVEs that no code change could clear (insight#2021). Floating minor
# tag on purpose — Docker Official Images rebuild it when Alpine ships package
# updates, so routine rebuilds pick the fixes up.
FROM nginxinc/nginx-unprivileged:1.31-alpine

# The base image already drops to an unprivileged user; the build steps below
# need to write under /etc/nginx, so root is restored for them and dropped again
# before the runtime stage.
USER root

COPY --from=builder /app/dist /usr/share/nginx/html
COPY docker-entrypoint.sh /docker-entrypoint.sh
RUN chmod +x /docker-entrypoint.sh

# nginx config: security-headers snippet (included at server level AND inside
# any location block that declares its own add_header — nginx replaces rather
# than merges across levels) + default.conf that the entrypoint renders into
# conf.d at container start.
RUN mkdir -p /etc/nginx/snippets
COPY nginx/security-headers.conf /etc/nginx/snippets/security-headers.conf
COPY nginx/default.conf.template /etc/nginx/templates/default.conf.template

USER nginx

EXPOSE 8080

ENTRYPOINT ["/docker-entrypoint.sh"]
CMD ["nginx", "-g", "daemon off;"]
