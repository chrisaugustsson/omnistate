# edge

One Traefik container that owns port 80 on this machine and routes to local
Docker Compose projects by hostname, so each project is reached at
`http://<name>.localhost` with no port number. OmniState is
`http://omnistate.localhost`.

## Run

From the repo root:

```bash
make edge
```

Leave it running; `restart: unless-stopped` brings it back with Docker Desktop.
The dashboard at <http://traefik.localhost> lists the hostnames it routes.

OmniState works without it, at the fallback `http://localhost:8420`.

## Add another project

Any project on the machine can join. In its `docker-compose.yml`, on the
service that should answer (its reverse proxy or web server):

```yaml
services:
  web:
    networks: [default, edge]
    labels:
      - traefik.enable=true
      - traefik.http.routers.${COMPOSE_PROJECT_NAME:-myproject}.rule=Host(`${COMPOSE_PROJECT_NAME:-myproject}.localhost`)
      - traefik.http.services.${COMPOSE_PROJECT_NAME:-myproject}.loadbalancer.server.port=3000
      - traefik.docker.network=edge

networks:
  edge:
    name: edge
```

`server.port` is the port the service listens on inside its container. Using
the Compose project name for the router and hostname means a second copy of
the stack (`docker compose -p myproject-test up`) gets `myproject-test.localhost`
of its own instead of being load-balanced with the first.

A project keeps working without the edge: Compose creates the `edge` network
itself when it is missing, and the labels are inert until Traefik reads them.
Keep any published port the project had as a fallback.

## Why `*.localhost`

macOS, Chrome, Firefox and curl resolve any `*.localhost` name to loopback
without `/etc/hosts`, and browsers treat those origins as secure contexts, so
`Secure` cookies work over plain http. Each hostname is its own cookie jar,
which `localhost:<port>` addresses are not: cookies ignore ports.
