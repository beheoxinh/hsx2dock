/**
 * Built-in compose project templates for the "New project" dialog.
 *
 * A template is a compose file plus the `.env` keys it reads. Secrets are
 * generated per project so two projects never share a database password.
 * The YAML leaves container names and host ports free of project-specific
 * values, so two projects from one template can run side by side only when
 * their ports differ; the dialog shows the ports so the user can change them.
 */

export interface ComposeTemplate {
  id: string;
  title: string;
  description: string;
  /** Shown under the YAML preview; what the folder still needs. */
  hint?: string;
  /** Compose file text; `${KEY}` refers to a value in the `.env` file. */
  yaml: string;
  /** `.env` keys filled with generated random passwords. */
  secrets: string[];
}

/** Joins YAML lines; keeps `${...}` free of JS template-literal escaping. */
const yaml = (...lines: string[]) => `${lines.join("\n")}\n`;

export const COMPOSE_TEMPLATES: ComposeTemplate[] = [
  {
    id: "nginx-static",
    title: "Static site (nginx)",
    description: "nginx serving files from the project folder on port 8080.",
    hint: "Put your site in ./site and mount it, or replace the image with your own build.",
    yaml: yaml(
      "services:",
      "  web:",
      "    image: nginx:1.27-alpine",
      "    ports:",
      '      - "8080:80"',
      "    volumes:",
      "      - ./site:/usr/share/nginx/html:ro",
      "    restart: unless-stopped"
    ),
    secrets: [],
  },
  {
    id: "web-postgres",
    title: "Web + PostgreSQL",
    description: "An app container next to PostgreSQL, started after the database is healthy.",
    hint: "Replace the nginx placeholder with your application image; it reads DATABASE_URL.",
    yaml: yaml(
      "services:",
      "  db:",
      "    image: postgres:16-alpine",
      "    environment:",
      "      POSTGRES_DB: app",
      "      POSTGRES_USER: app",
      "      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}",
      "    volumes:",
      "      - db-data:/var/lib/postgresql/data",
      "    healthcheck:",
      '      test: ["CMD-SHELL", "pg_isready -U app -d app"]',
      "      interval: 5s",
      "      timeout: 3s",
      "      retries: 10",
      "  web:",
      "    image: nginx:1.27-alpine",
      "    ports:",
      '      - "8080:80"',
      "    environment:",
      "      DATABASE_URL: postgres://app:${POSTGRES_PASSWORD}@db:5432/app",
      "    depends_on:",
      "      db:",
      "        condition: service_healthy",
      "volumes:",
      "  db-data:"
    ),
    secrets: ["POSTGRES_PASSWORD"],
  },
  {
    id: "wordpress-mariadb",
    title: "WordPress + MariaDB",
    description: "WordPress on port 8081 with a MariaDB database.",
    yaml: yaml(
      "services:",
      "  db:",
      "    image: mariadb:11",
      "    environment:",
      "      MARIADB_DATABASE: wordpress",
      "      MARIADB_USER: wordpress",
      "      MARIADB_PASSWORD: ${MARIADB_PASSWORD}",
      "      MARIADB_ROOT_PASSWORD: ${MARIADB_ROOT_PASSWORD}",
      "    volumes:",
      "      - db-data:/var/lib/mysql",
      "  wordpress:",
      "    image: wordpress:6-php8.3-apache",
      "    ports:",
      '      - "8081:80"',
      "    environment:",
      "      WORDPRESS_DB_HOST: db",
      "      WORDPRESS_DB_USER: wordpress",
      "      WORDPRESS_DB_PASSWORD: ${MARIADB_PASSWORD}",
      "      WORDPRESS_DB_NAME: wordpress",
      "    volumes:",
      "      - wp-data:/var/www/html",
      "    depends_on:",
      "      - db",
      "volumes:",
      "  db-data:",
      "  wp-data:"
    ),
    secrets: ["MARIADB_PASSWORD", "MARIADB_ROOT_PASSWORD"],
  },
  {
    id: "postgres-pgadmin",
    title: "PostgreSQL + pgAdmin",
    description: "PostgreSQL with the pgAdmin web UI on port 5050.",
    hint: "Log in to pgAdmin with the generated PGADMIN_PASSWORD from the .env file.",
    yaml: yaml(
      "services:",
      "  postgres:",
      "    image: postgres:16-alpine",
      "    environment:",
      "      POSTGRES_DB: app",
      "      POSTGRES_USER: app",
      "      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}",
      "    volumes:",
      "      - pg-data:/var/lib/postgresql/data",
      "  pgadmin:",
      "    image: dpage/pgadmin4:8",
      "    ports:",
      '      - "5050:80"',
      "    environment:",
      "      PGADMIN_DEFAULT_EMAIL: admin@example.test",
      "      PGADMIN_DEFAULT_PASSWORD: ${PGADMIN_PASSWORD}",
      "    depends_on:",
      "      - postgres",
      "volumes:",
      "  pg-data:"
    ),
    secrets: ["POSTGRES_PASSWORD", "PGADMIN_PASSWORD"],
  },
  {
    id: "redis",
    title: "Redis",
    description: "Redis with a password and append-only persistence, bound to localhost.",
    yaml: yaml(
      "services:",
      "  redis:",
      "    image: redis:7-alpine",
      '    command: ["redis-server", "--requirepass", "${REDIS_PASSWORD}", "--appendonly", "yes"]',
      "    ports:",
      '      - "127.0.0.1:6379:6379"',
      "    volumes:",
      "      - redis-data:/data",
      "    restart: unless-stopped",
      "volumes:",
      "  redis-data:"
    ),
    secrets: ["REDIS_PASSWORD"],
  },
  {
    id: "node-watch",
    title: "Node app with build and watch",
    description: "Built from a Dockerfile in the folder; `compose watch` syncs ./src and rebuilds on package.json changes.",
    hint: "Add a Dockerfile next to the compose file. Then use Start watch from the project menu.",
    yaml: yaml(
      "services:",
      "  app:",
      "    build: .",
      "    ports:",
      '      - "3000:3000"',
      "    environment:",
      "      NODE_ENV: development",
      "    develop:",
      "      watch:",
      "        - action: sync",
      "          path: ./src",
      "          target: /app/src",
      "        - action: rebuild",
      "          path: ./package.json"
    ),
    secrets: [],
  },
];

/** Letters and digits only: safe unquoted in `.env` and in a shell. */
export const PASSWORD_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/** Crypto-grade random bytes from the platform (browser and Node 19+). */
export function randomBytes(n: number): Uint8Array {
  return globalThis.crypto.getRandomValues(new Uint8Array(n));
}

/**
 * A random password of `length` characters from `PASSWORD_ALPHABET`.
 * Bytes at or above the largest multiple of the alphabet size are rejected,
 * so every character is equally likely (a plain `% 62` would favour the
 * first eight characters). `bytes` is injectable for tests.
 */
export function generatePassword(length = 24, bytes: (n: number) => Uint8Array = randomBytes): string {
  const size = PASSWORD_ALPHABET.length;
  const limit = 256 - (256 % size);
  let out = "";
  while (out.length < length) {
    for (const b of bytes(length)) {
      if (b >= limit) continue;
      out += PASSWORD_ALPHABET[b % size];
      if (out.length === length) break;
    }
  }
  return out;
}

/** Generated values for a template's secret keys. */
export function templateSecrets(t: ComposeTemplate, gen: () => string = () => generatePassword()): Record<string, string> {
  const values: Record<string, string> = {};
  for (const key of t.secrets) values[key] = gen();
  return values;
}

/** `.env` text: one `KEY=value` line per entry, with a header comment. */
export function formatEnvFile(values: Record<string, string>): string {
  const keys = Object.keys(values);
  if (keys.length === 0) return "";
  return [
    "# Generated by hsx2dock. Keep this file out of version control.",
    ...keys.map((k) => `${k}=${values[k]}`),
  ].join("\n") + "\n";
}

/**
 * A compose project name from free text: lowercase letters, digits, `-`
 * and `_`, starting with a letter or digit. Empty input becomes "app".
 */
export function slugProjectName(raw: string): string {
  const slug = raw
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^[^a-z0-9]+/, "")
    .replace(/-+$/, "");
  return slug || "app";
}

/** Default folder for a new project: `~/compose/<name>`. */
export function defaultProjectDir(name: string): string {
  return `~/compose/${slugProjectName(name)}`;
}
