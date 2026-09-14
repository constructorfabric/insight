FROM node:24-bookworm-slim@sha256:2fe369e969550cde8e867afc3fe370b260140cab4a23d467074295b42163d553 AS builder

WORKDIR /app

COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./

# Not corepack: node images past 24 no longer ship it, and its package collides
# with the yarn bins this image already holds.
RUN npm install -g "pnpm@$(node -p 'require("/app/package.json").packageManager.split("@")[1]')"

RUN pnpm install --frozen-lockfile --ignore-scripts

COPY . .

ARG VITE_APP_RELEASE

RUN pnpm run build

# Runtime base: keep this on a currently-maintained nginx line. The 1.27 tag
# stopped getting Alpine package refreshes, so the published image accumulated
# fixable OS CVEs that no code change could clear (insight#2021). Floating minor
# tag on purpose — Docker Official Images rebuild it when Alpine ships package
# updates, so routine rebuilds pick the fixes up.
FROM nginxinc/nginx-unprivileged:1.31-alpine@sha256:19c132c9ab02d3b783f478743dafc7a7f42e27aa7d2bdcbec1bb1128ca8f2a07

# The base image already drops to an unprivileged user; the build steps below
# need to write under /etc/nginx, so root is restored for them and dropped again
# before the runtime stage.
USER root

RUN apk upgrade --no-cache libcrypto3 libssl3

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
