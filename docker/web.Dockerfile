FROM node:22-alpine AS build
RUN npm install --global pnpm@10
WORKDIR /app
COPY pnpm-workspace.yaml pnpm-lock.yaml ./
COPY web/package.json web/
RUN pnpm install --frozen-lockfile
COPY web web
RUN pnpm --filter omni-web build

FROM node:22-alpine
WORKDIR /app
COPY --from=build /app/web/.output .output
ENV HOST=0.0.0.0 PORT=3000
CMD ["node", ".output/server/index.mjs"]
