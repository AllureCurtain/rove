import { describe, expect, it, vi } from "vitest";

import {
  M1_BROWSER_MIGRATION_STATE_KEY,
  M1_BROWSER_STORAGE_KEYS,
} from "./m1-storage-keys";
import {
  readM1BrowserMigrationState,
  runM1BrowserMigration,
  type M1BrowserMigrationLock,
  type MigrationStorage,
} from "./m1-browser-migration";
import {
  parseM1BrowserMigrationRequest,
  type M1BrowserMigrationRequest,
  type M1BrowserMigrationResponse,
} from "./product-api-types";
import {
  createProductApiClient,
  type ProductApiClient,
} from "./product-client";

class MemoryStorage implements MigrationStorage {
  readonly reads: string[] = [];
  readonly writes: string[] = [];
  readonly writtenValues: string[] = [];
  readonly removes: string[] = [];
  private readonly values = new Map<string, string>();

  constructor(initial: Record<string, string> = {}) {
    for (const [key, value] of Object.entries(initial)) {
      this.values.set(key, value);
    }
  }

  getItem(key: string): string | null {
    this.reads.push(key);
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.writes.push(key);
    this.writtenValues.push(value);
    this.values.set(key, value);
  }

  removeItem(key: string): void {
    this.removes.push(key);
    this.values.delete(key);
  }

  peek(key: string): string | null {
    return this.values.get(key) ?? null;
  }
}

const immediateMigrationLock: M1BrowserMigrationLock = {
  runExclusive<T>(_name: string, operation: () => Promise<T>): Promise<T> {
    return operation();
  },
};

class SerialMigrationLock implements M1BrowserMigrationLock {
  private tail: Promise<void> = Promise.resolve();

  async runExclusive<T>(
    _name: string,
    operation: () => Promise<T>,
  ): Promise<T> {
    const previous = this.tail;
    let release = (): void => undefined;
    this.tail = new Promise<void>((resolve) => {
      release = resolve;
    });
    await previous;
    try {
      return await operation();
    } finally {
      release();
    }
  }
}

function legacyState(
  profileExtra: Record<string, unknown> = {},
): Record<string, string> {
  return {
    [M1_BROWSER_STORAGE_KEYS.workspaces]: JSON.stringify([
      {
        id: "ws_legacy",
        rootPath: "D:\\Study\\project\\agent\\rove",
        kind: "repo",
        displayName: "rove",
        pinned: true,
        lastOpenedAt: "2026-07-25T10:00:00.000Z",
        ignoredOldField: "never forwarded",
      },
    ]),
    [M1_BROWSER_STORAGE_KEYS.sessions]: JSON.stringify([
      {
        id: "sess_legacy",
        workspaceId: "ws_legacy",
        title: "Migration test",
        createdAt: "2026-07-25T10:00:00.000Z",
        updatedAt: "2026-07-25T11:00:00.000Z",
        status: "idle",
        activeJobId: "job_legacy",
        activeRunId: "run_legacy",
        resumedFromRunId: null,
        hasDurableTurn: true,
      },
    ]),
    [M1_BROWSER_STORAGE_KEYS.active]: JSON.stringify({
      workspaceId: "ws_legacy",
      sessionId: "sess_legacy",
    }),
    [M1_BROWSER_STORAGE_KEYS.providerProfiles]: JSON.stringify([
      {
        id: "prov_legacy",
        label: "Gateway",
        providerType: "openai",
        apiBase: "https://gateway.example.test/v1",
        apiKeyEnv: "GATEWAY_API_KEY",
        defaultModel: "test/model",
        updatedAt: "2026-07-25T11:00:00.000Z",
        ...profileExtra,
      },
    ]),
    [M1_BROWSER_STORAGE_KEYS.providerSelection]: JSON.stringify({
      mode: "profile",
      profileId: "prov_legacy",
      model: "test/model",
      approval: "ask",
      maxSteps: 8,
    }),
    [M1_BROWSER_STORAGE_KEYS.theme]: "dark",
  };
}

function acknowledgementFor(
  body: string,
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  const request: M1BrowserMigrationRequest = parseM1BrowserMigrationRequest(
    JSON.parse(body),
  );
  return {
    source_schema_version: request.source_schema_version,
    idempotency_key: request.idempotency_key,
    receipt_id: "01J00000000000000000000000",
    disposition: "applied",
    workspace_mappings: request.workspaces.map((workspace) => ({
      source_id: workspace.source_id,
      workspace_id: "01J00000000000000000000001",
    })),
    session_mappings: request.sessions.map((session) => ({
      source_id: session.source_id,
      product_session_id: "01J00000000000000000000002",
    })),
    provider_profile_mappings: request.provider_profiles.map((profile) => ({
      source_id: profile.source_id,
      provider_profile_id: "01J00000000000000000000003",
    })),
    issues: [],
    applied_at: "2026-07-26T00:00:01.000Z",
    ...overrides,
  };
}

function successfulFetch(
  inspect?: (body: string) => void,
): typeof globalThis.fetch {
  return vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
    const body = String(init?.body ?? "");
    inspect?.(body);
    return new Response(JSON.stringify(acknowledgementFor(body)), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  });
}

describe("M1 browser migration", () => {
  it("does nothing when no M1 source key or migration state exists", async () => {
    const storage = new MemoryStorage();
    const fetchMock = vi.fn();
    const idGenerator = vi.fn(() => "migration-not-needed");
    const now = vi.fn(() => "2026-07-26T00:00:00.000Z");

    const result = await runM1BrowserMigration({
      storage,
      lock: immediateMigrationLock,
      fetch: fetchMock,
      idGenerator,
      now,
    });

    expect(result).toEqual({ status: "not_needed" });
    expect(fetchMock).not.toHaveBeenCalled();
    expect(idGenerator).not.toHaveBeenCalled();
    expect(now).not.toHaveBeenCalled();
    expect(storage.writes).toEqual([]);
    expect(storage.peek(M1_BROWSER_MIGRATION_STATE_KEY)).toBeNull();
  });

  it("does not migrate the empty defaults written by an M1 shell mount", async () => {
    const storage = new MemoryStorage({
      [M1_BROWSER_STORAGE_KEYS.workspaces]: JSON.stringify([]),
      [M1_BROWSER_STORAGE_KEYS.sessions]: JSON.stringify([]),
      [M1_BROWSER_STORAGE_KEYS.active]: JSON.stringify({
        workspaceId: null,
        sessionId: null,
      }),
      [M1_BROWSER_STORAGE_KEYS.providerProfiles]: JSON.stringify([]),
      [M1_BROWSER_STORAGE_KEYS.providerSelection]: JSON.stringify({
        mode: "default",
        model: "fake",
        approval: "ask",
        maxSteps: 8,
      }),
      [M1_BROWSER_STORAGE_KEYS.theme]: "light",
    });
    const fetchMock = vi.fn();
    const idGenerator = vi.fn(() => "migration-empty-shell");

    const result = await runM1BrowserMigration({
      storage,
      lock: immediateMigrationLock,
      fetch: fetchMock,
      idGenerator,
    });

    expect(result).toEqual({ status: "not_needed" });
    expect(fetchMock).not.toHaveBeenCalled();
    expect(idGenerator).not.toHaveBeenCalled();
    expect(storage.writes).toEqual([]);
    expect(storage.removes).toEqual([]);
  });

  it("fails closed without same-origin locking before touching browser state", async () => {
    const storage = new MemoryStorage(legacyState());
    const fetchMock = vi.fn();
    const idGenerator = vi.fn(() => "migration-without-lock");

    const result = await runM1BrowserMigration({
      storage,
      lock: null,
      fetch: fetchMock,
      idGenerator,
    });

    expect(result).toEqual({
      status: "blocked",
      failure: {
        code: "lock_unavailable",
        message: "M1 browser migration requires same-origin exclusive locking",
      },
    });
    expect(fetchMock).not.toHaveBeenCalled();
    expect(idGenerator).not.toHaveBeenCalled();
    expect(storage.reads).toEqual([]);
    expect(storage.writes).toEqual([]);
  });

  it("constructs an allowlisted body and never forwards raw secret-shaped fields", async () => {
    const storage = new MemoryStorage(
      legacyState({
        apiKey: "sk-must-not-leave-browser",
        key: "raw-key",
        token: "raw-token",
        authorization: "Bearer raw",
      }),
    );
    let postedBody = "";

    const result = await runM1BrowserMigration({
      storage,
      lock: immediateMigrationLock,
      fetch: successfulFetch((body) => {
        postedBody = body;
      }),
      idGenerator: () => "migration-allowlist",
      now: () => "2026-07-26T00:00:00.000Z",
    });

    expect(result.status).toBe("complete");
    expect(postedBody).toContain('"api_key_env":"GATEWAY_API_KEY"');
    expect(postedBody).not.toMatch(/apiKey|sk-must|raw-key|raw-token|authorization/i);
    expect(postedBody).not.toContain("ignoredOldField");
  });

  it("omits theme when the legacy theme key is missing", async () => {
    const initial = legacyState();
    delete initial[M1_BROWSER_STORAGE_KEYS.theme];
    const storage = new MemoryStorage(initial);
    let postedRequest: M1BrowserMigrationRequest | undefined;

    const result = await runM1BrowserMigration({
      storage,
      lock: immediateMigrationLock,
      fetch: successfulFetch((body) => {
        postedRequest = parseM1BrowserMigrationRequest(JSON.parse(body));
      }),
      idGenerator: () => "migration-without-theme",
      now: () => "2026-07-26T00:00:00.000Z",
    });

    expect(result.status).toBe("complete");
    expect(postedRequest).toBeDefined();
    expect(postedRequest?.safe_preferences).not.toHaveProperty("theme");
  });

  it("does not invent optional preferences beside workspace data and shell defaults", async () => {
    const fullState = legacyState();
    const storage = new MemoryStorage({
      [M1_BROWSER_STORAGE_KEYS.workspaces]:
        fullState[M1_BROWSER_STORAGE_KEYS.workspaces]!,
      [M1_BROWSER_STORAGE_KEYS.providerSelection]: JSON.stringify({
        mode: "default",
        model: "fake",
        approval: "ask",
        maxSteps: 8,
      }),
      [M1_BROWSER_STORAGE_KEYS.theme]: "light",
    });
    let postedRequest: M1BrowserMigrationRequest | undefined;

    const result = await runM1BrowserMigration({
      storage,
      lock: immediateMigrationLock,
      fetch: successfulFetch((body) => {
        postedRequest = parseM1BrowserMigrationRequest(JSON.parse(body));
      }),
      idGenerator: () => "migration-partial-legacy-state",
      now: () => "2026-07-26T00:00:00.000Z",
    });

    expect(result.status).toBe("complete");
    expect(postedRequest).toMatchObject({
      sessions: [],
      provider_profiles: [],
      safe_preferences: {},
    });
    expect(postedRequest?.workspaces).toHaveLength(1);
    expect(Object.keys(postedRequest?.safe_preferences ?? {})).toEqual([]);
  });

  it("blocks malformed JSON and wrong legacy root types without posting", async () => {
    const malformed = new MemoryStorage({
      ...legacyState(),
      [M1_BROWSER_STORAGE_KEYS.workspaces]: "{not-json",
    });
    const malformedFetch = vi.fn();
    const malformedResult = await runM1BrowserMigration({
      storage: malformed,
      lock: immediateMigrationLock,
      fetch: malformedFetch,
      idGenerator: () => "migration-malformed",
    });

    expect(malformedResult.status).toBe("blocked");
    expect(malformedFetch).not.toHaveBeenCalled();
    expect(malformed.peek(M1_BROWSER_MIGRATION_STATE_KEY)).toBeNull();

    const wrongRoot = new MemoryStorage({
      ...legacyState(),
      [M1_BROWSER_STORAGE_KEYS.sessions]: JSON.stringify({ session: [] }),
    });
    const wrongRootFetch = vi.fn();
    const wrongRootResult = await runM1BrowserMigration({
      storage: wrongRoot,
      lock: immediateMigrationLock,
      fetch: wrongRootFetch,
      idGenerator: () => "migration-wrong-root",
    });

    expect(wrongRootResult.status).toBe("blocked");
    expect(wrongRootFetch).not.toHaveBeenCalled();
  });

  it.each([
    {
      name: "empty session title",
      key: M1_BROWSER_STORAGE_KEYS.sessions,
      field: "title",
      value: "",
    },
    {
      name: "session title with a control character",
      key: M1_BROWSER_STORAGE_KEYS.sessions,
      field: "title",
      value: "line\nbreak",
    },
    {
      name: "non-RFC3339 workspace timestamp",
      key: M1_BROWSER_STORAGE_KEYS.workspaces,
      field: "lastOpenedAt",
      value: "yesterday",
    },
    {
      name: "oversized session timestamp",
      key: M1_BROWSER_STORAGE_KEYS.sessions,
      field: "createdAt",
      value: "x".repeat(513),
    },
    {
      name: "invalid calendar date",
      key: M1_BROWSER_STORAGE_KEYS.sessions,
      field: "updatedAt",
      value: "2026-02-30T00:00:00.000Z",
    },
    {
      name: "non-RFC3339 provider timestamp",
      key: M1_BROWSER_STORAGE_KEYS.providerProfiles,
      field: "updatedAt",
      value: "not-a-timestamp",
    },
  ])("rejects $name before posting", async ({ key, field, value }) => {
    const initial = legacyState();
    const records = JSON.parse(initial[key]!) as Array<Record<string, unknown>>;
    records[0]![field] = value;
    initial[key] = JSON.stringify(records);
    const storage = new MemoryStorage(initial);
    const fetchMock = vi.fn();

    const result = await runM1BrowserMigration({
      storage,
      lock: immediateMigrationLock,
      fetch: fetchMock,
      idGenerator: () => "migration-invalid-client-data",
    });

    expect(result.status).toBe("blocked");
    expect(fetchMock).not.toHaveBeenCalled();
    expect(storage.peek(M1_BROWSER_MIGRATION_STATE_KEY)).toBeNull();
  });

  it.each(["local", ""])(
    "normalizes trusted legacy fake base %j to the server canonical empty value",
    async (legacyApiBase) => {
      const storage = new MemoryStorage(
        legacyState({
          providerType: "fake",
          apiBase: legacyApiBase,
          apiKeyEnv: undefined,
        }),
      );
      let postedBody = "";

      const result = await runM1BrowserMigration({
        storage,
        lock: immediateMigrationLock,
        fetch: successfulFetch((body) => {
          postedBody = body;
        }),
        idGenerator: () => "migration-fake-base",
        now: () => "2026-07-26T00:00:00.000Z",
      });

      expect(result.status).toBe("complete");
      const postedRequest: M1BrowserMigrationRequest =
        parseM1BrowserMigrationRequest(JSON.parse(postedBody));
      expect(postedRequest.provider_profiles[0]?.api_base).toBe("");
    },
  );

  it("fails closed for a known M1 task workspace that C0 cannot import", async () => {
    const initial = legacyState();
    const workspaces = JSON.parse(
      initial[M1_BROWSER_STORAGE_KEYS.workspaces]!,
    ) as Array<Record<string, unknown>>;
    workspaces[0]!.kind = "task";
    initial[M1_BROWSER_STORAGE_KEYS.workspaces] = JSON.stringify(workspaces);
    const storage = new MemoryStorage(initial);
    const fetchMock = vi.fn();

    const result = await runM1BrowserMigration({
      storage,
      lock: immediateMigrationLock,
      fetch: fetchMock,
      idGenerator: () => "migration-task-workspace",
    });

    expect(result.status).toBe("blocked");
    expect(fetchMock).not.toHaveBeenCalled();
    expect(storage.peek(M1_BROWSER_MIGRATION_STATE_KEY)).toBeNull();
    for (const [key, value] of Object.entries(initial)) {
      expect(storage.peek(key)).toBe(value);
    }
  });

  it("persists the exact pending request before POST", async () => {
    const storage = new MemoryStorage(legacyState());
    let observedPendingBody = "";

    const result = await runM1BrowserMigration({
      storage,
      lock: immediateMigrationLock,
      fetch: successfulFetch((body) => {
        const state = readM1BrowserMigrationState(storage);
        expect(state?.status).toBe("pending");
        if (state?.status === "pending") {
          observedPendingBody = state.request_body;
          expect(state.idempotency_key).toBe("migration-pending-first");
          expect(state.request_body).toBe(body);
        }
      }),
      idGenerator: () => "migration-pending-first",
      now: () => "2026-07-26T00:00:00.000Z",
    });

    expect(result.status).toBe("complete");
    expect(observedPendingBody).not.toBe("");
  });

  it("serializes concurrent tabs across the complete migration transaction", async () => {
    const storage = new MemoryStorage(legacyState());
    const lock = new SerialMigrationLock();
    const fetchMock = successfulFetch();
    const idGenerator = vi.fn(() => "migration-concurrent");

    const [first, second] = await Promise.all([
      runM1BrowserMigration({
        storage,
        lock,
        fetch: fetchMock,
        idGenerator,
        now: () => "2026-07-26T00:00:00.000Z",
      }),
      runM1BrowserMigration({
        storage,
        lock,
        fetch: fetchMock,
        idGenerator,
        now: () => "2026-07-26T00:00:02.000Z",
      }),
    ]);

    expect(first.status).toBe("complete");
    expect(second.status).toBe("complete");
    if (first.status === "complete" && second.status === "complete") {
      expect(first.reused).toBe(false);
      expect(second.reused).toBe(true);
      expect(second.state).toEqual(first.state);
    }
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(idGenerator).toHaveBeenCalledTimes(1);
    expect(storage.writes).toEqual([
      M1_BROWSER_MIGRATION_STATE_KEY,
      M1_BROWSER_MIGRATION_STATE_KEY,
    ]);
    expect(
      storage.writtenValues.map((raw) =>
        (JSON.parse(raw) as { status?: unknown }).status,
      ),
    ).toEqual(["pending", "complete"]);
    expect(readM1BrowserMigrationState(storage)?.status).toBe("complete");
  });

  it("replays the exact persisted body and key after a lost response", async () => {
    const storage = new MemoryStorage(legacyState());
    const postedBodies: string[] = [];
    const idGenerator = vi.fn(() => "migration-replay");
    const lostResponseFetch = vi.fn(
      async (_input: RequestInfo | URL, init?: RequestInit) => {
        postedBodies.push(String(init?.body ?? ""));
        throw new TypeError("connection closed after server commit");
      },
    );

    const first = await runM1BrowserMigration({
      storage,
      lock: immediateMigrationLock,
      fetch: lostResponseFetch,
      idGenerator,
      now: () => "2026-07-26T00:00:00.000Z",
    });
    expect(first.status).toBe("pending");

    storage.setItem(
      M1_BROWSER_STORAGE_KEYS.providerProfiles,
      JSON.stringify([]),
    );
    const second = await runM1BrowserMigration({
      storage,
      lock: immediateMigrationLock,
      fetch: successfulFetch((body) => postedBodies.push(body)),
      idGenerator,
      now: () => "2026-07-26T00:00:02.000Z",
    });

    expect(second.status).toBe("complete");
    expect(postedBodies).toHaveLength(2);
    expect(postedBodies[1]).toBe(postedBodies[0]);
    expect(postedBodies[1]).toContain('"idempotency_key":"migration-replay"');
    expect(idGenerator).toHaveBeenCalledTimes(1);
  });

  it("exactly replays a pending request created by the required-theme client", async () => {
    const request: M1BrowserMigrationRequest = {
      source: "web_m1_local_storage",
      source_schema_version: 1,
      idempotency_key: "migration-required-theme-pending",
      workspaces: [],
      sessions: [],
      provider_profiles: [],
      safe_preferences: { theme: "light" },
    };
    const requestBody = JSON.stringify(request);
    const storage = new MemoryStorage({
      [M1_BROWSER_MIGRATION_STATE_KEY]: JSON.stringify({
        status: "pending",
        source_schema_version: 1,
        idempotency_key: request.idempotency_key,
        request,
        request_body: requestBody,
        created_at: "2026-07-25T00:00:00.000Z",
      }),
    });
    const idGenerator = vi.fn(() => "migration-must-not-replace-pending");
    let postedBody = "";

    const result = await runM1BrowserMigration({
      storage,
      lock: immediateMigrationLock,
      fetch: successfulFetch((body) => {
        postedBody = body;
      }),
      idGenerator,
      now: () => "2026-07-26T00:00:00.000Z",
    });

    expect(result.status).toBe("complete");
    expect(postedBody).toBe(requestBody);
    expect(postedBody).toContain('"safe_preferences":{"theme":"light"}');
    expect(idGenerator).not.toHaveBeenCalled();
  });

  it.each([408, 429, 500])(
    "keeps and exactly replays pending after HTTP %i",
    async (status) => {
      const storage = new MemoryStorage(legacyState());
      const postedBodies: string[] = [];
      const idGenerator = vi.fn(() => `migration-server-replay-${status}`);
      const unavailableFetch = vi.fn(
        async (_input: RequestInfo | URL, init?: RequestInit) => {
          postedBodies.push(String(init?.body ?? ""));
          return new Response(
            JSON.stringify({
              code: "product_store_unavailable",
              error: "temporarily unavailable",
            }),
            { status },
          );
        },
      );

      const first = await runM1BrowserMigration({
        storage,
        lock: immediateMigrationLock,
        fetch: unavailableFetch,
        idGenerator,
        now: () => "2026-07-26T00:00:00.000Z",
      });
      expect(first.status).toBe("pending");

      storage.setItem(M1_BROWSER_STORAGE_KEYS.theme, "system");
      const second = await runM1BrowserMigration({
        storage,
        lock: immediateMigrationLock,
        fetch: successfulFetch((body) => postedBodies.push(body)),
        idGenerator,
        now: () => "2026-07-26T00:00:02.000Z",
      });

      expect(second.status).toBe("complete");
      expect(postedBodies).toHaveLength(2);
      expect(postedBodies[1]).toBe(postedBodies[0]);
      expect(idGenerator).toHaveBeenCalledTimes(1);
      expect(storage.removes).toEqual([]);
    },
  );

  it.each([400, 409])(
    "clears an exact pending rejected with %i and retries corrected legacy state with a new key",
    async (status) => {
      const initial = legacyState();
      const storage = new MemoryStorage(initial);
      const postedBodies: string[] = [];
      const idGenerator = vi
        .fn(() => "migration-unused")
        .mockReturnValueOnce(`migration-rejected-${status}`)
        .mockReturnValueOnce(`migration-corrected-${status}`);
      const rejectedFetch = vi.fn(
        async (_input: RequestInfo | URL, init?: RequestInit) => {
          postedBodies.push(String(init?.body ?? ""));
          return new Response(
            JSON.stringify({
              code:
                status === 409
                  ? "migration_idempotency_conflict"
                  : "product_invalid_input",
              error: "migration rejected",
            }),
            { status },
          );
        },
      );

      const first = await runM1BrowserMigration({
        storage,
        lock: immediateMigrationLock,
        fetch: rejectedFetch,
        idGenerator,
        now: () => "2026-07-26T00:00:00.000Z",
      });

      expect(first.status).toBe("rejected");
      expect(readM1BrowserMigrationState(storage)).toBeNull();
      expect(storage.removes).toEqual([M1_BROWSER_MIGRATION_STATE_KEY]);
      for (const [key, value] of Object.entries(initial)) {
        expect(storage.peek(key)).toBe(value);
      }

      const sessions = JSON.parse(
        storage.peek(M1_BROWSER_STORAGE_KEYS.sessions)!,
      ) as Array<Record<string, unknown>>;
      sessions[0]!.title = "Corrected migration title";
      storage.setItem(
        M1_BROWSER_STORAGE_KEYS.sessions,
        JSON.stringify(sessions),
      );
      const second = await runM1BrowserMigration({
        storage,
        lock: immediateMigrationLock,
        fetch: successfulFetch((body) => postedBodies.push(body)),
        idGenerator,
        now: () => "2026-07-26T00:00:02.000Z",
      });

      expect(second.status).toBe("complete");
      expect(postedBodies).toHaveLength(2);
      expect(postedBodies[0]).toContain(`migration-rejected-${status}`);
      expect(postedBodies[1]).toContain(`migration-corrected-${status}`);
      expect(postedBodies[1]).toContain("Corrected migration title");
      expect(idGenerator).toHaveBeenCalledTimes(2);
    },
  );

  it("keeps pending when deterministic rejection state clearing fails", async () => {
    class FailingRemoveStorage extends MemoryStorage {
      override removeItem(key: string): void {
        this.removes.push(key);
        throw new Error("storage unavailable");
      }
    }
    const storage = new FailingRemoveStorage(legacyState());

    const result = await runM1BrowserMigration({
      storage,
      lock: immediateMigrationLock,
      fetch: vi.fn(async () =>
        new Response(
          JSON.stringify({
            code: "product_invalid_input",
            error: "migration rejected",
          }),
          { status: 400 },
        ),
      ),
      idGenerator: () => "migration-rejected-clear-failure",
      now: () => "2026-07-26T00:00:00.000Z",
    });

    expect(result.status).toBe("pending");
    if (result.status === "pending") {
      expect(result.failure.code).toBe("storage_write_failed");
    }
    expect(readM1BrowserMigrationState(storage)?.status).toBe("pending");
    expect(storage.removes).toEqual([M1_BROWSER_MIGRATION_STATE_KEY]);
  });

  it("times out a hanging injected client, releases the lock, and exactly replays pending", async () => {
    vi.useFakeTimers();
    try {
      const storage = new MemoryStorage(legacyState());
      const lock = new SerialMigrationLock();
      const idGenerator = vi.fn(() => "migration-timeout-replay");
      let hangingBody = "";
      const hangingClient: ProductApiClient = {
        ...createProductApiClient({ fetch: vi.fn() }),
        migrateM1BrowserState(exact) {
          hangingBody = exact.body;
          return new Promise<M1BrowserMigrationResponse>(() => undefined);
        },
      };

      const firstPromise = runM1BrowserMigration({
        storage,
        lock,
        client: hangingClient,
        idGenerator,
        now: () => "2026-07-26T00:00:00.000Z",
        requestTimeoutMs: 25,
      });
      await vi.advanceTimersByTimeAsync(25);
      const first = await firstPromise;

      expect(first.status).toBe("pending");
      if (first.status === "pending") {
        expect(first.failure.message).toContain("timed out");
      }
      expect(vi.getTimerCount()).toBe(0);

      let replayedBody = "";
      const second = await runM1BrowserMigration({
        storage,
        lock,
        fetch: successfulFetch((body) => {
          replayedBody = body;
        }),
        idGenerator,
        now: () => "2026-07-26T00:00:02.000Z",
        requestTimeoutMs: 25,
      });

      expect(second.status).toBe("complete");
      expect(replayedBody).toBe(hangingBody);
      expect(idGenerator).toHaveBeenCalledTimes(1);
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps pending when the acknowledgement is invalid", async () => {
    const storage = new MemoryStorage(legacyState());
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, init?: RequestInit) => {
        const body = String(init?.body ?? "");
        return new Response(
          JSON.stringify(
            acknowledgementFor(body, {
              idempotency_key: "different-key",
            }),
          ),
          { status: 200 },
        );
      },
    );

    const result = await runM1BrowserMigration({
      storage,
      lock: immediateMigrationLock,
      fetch: fetchMock,
      idGenerator: () => "migration-invalid-ack",
      now: () => "2026-07-26T00:00:00.000Z",
    });

    expect(result.status).toBe("pending");
    expect(readM1BrowserMigrationState(storage)?.status).toBe("pending");
  });

  it("does not complete when an acknowledgement silently omits entity mappings", async () => {
    const storage = new MemoryStorage(legacyState());
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, init?: RequestInit) => {
        const body = String(init?.body ?? "");
        return new Response(
          JSON.stringify(
            acknowledgementFor(body, {
              workspace_mappings: [],
              session_mappings: [],
              provider_profile_mappings: [],
              issues: [],
            }),
          ),
          { status: 200 },
        );
      },
    );

    const result = await runM1BrowserMigration({
      storage,
      lock: immediateMigrationLock,
      fetch: fetchMock,
      idGenerator: () => "migration-incomplete-ack",
      now: () => "2026-07-26T00:00:00.000Z",
    });

    expect(result.status).toBe("pending");
    expect(readM1BrowserMigrationState(storage)?.status).toBe("pending");
  });

  it.each([
    {
      name: "active workspace",
      mutate(initial: Record<string, string>): void {
        initial[M1_BROWSER_STORAGE_KEYS.active] = JSON.stringify({
          workspaceId: "ws_missing",
          sessionId: "sess_legacy",
        });
      },
    },
    {
      name: "active session",
      mutate(initial: Record<string, string>): void {
        initial[M1_BROWSER_STORAGE_KEYS.active] = JSON.stringify({
          workspaceId: "ws_legacy",
          sessionId: "sess_missing",
        });
      },
    },
    {
      name: "provider selection",
      mutate(initial: Record<string, string>): void {
        initial[M1_BROWSER_STORAGE_KEYS.providerSelection] = JSON.stringify({
          mode: "profile",
          profileId: "prov_missing",
          model: "test/model",
          approval: "ask",
          maxSteps: 8,
        });
      },
    },
  ])(
    "keeps pending when the $name reference has neither a mapping nor an issue",
    async ({ mutate }) => {
      const initial = legacyState();
      mutate(initial);
      const storage = new MemoryStorage(initial);

      const result = await runM1BrowserMigration({
        storage,
        lock: immediateMigrationLock,
        fetch: successfulFetch(),
        idGenerator: () => "migration-invalid-preference-ack",
        now: () => "2026-07-26T00:00:00.000Z",
      });

      expect(result.status).toBe("pending");
      if (result.status === "pending") {
        expect(result.failure.code).toBe("invalid_acknowledgement");
      }
      expect(readM1BrowserMigrationState(storage)?.status).toBe("pending");
    },
  );

  it("accepts exact issues for preference references without mappings", async () => {
    const initial = legacyState();
    initial[M1_BROWSER_STORAGE_KEYS.active] = JSON.stringify({
      workspaceId: "ws_missing",
      sessionId: "sess_missing",
    });
    initial[M1_BROWSER_STORAGE_KEYS.providerSelection] = JSON.stringify({
      mode: "profile",
      profileId: "prov_missing",
      model: "test/model",
      approval: "ask",
      maxSteps: 8,
    });
    const storage = new MemoryStorage(initial);
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, init?: RequestInit) => {
        const body = String(init?.body ?? "");
        return new Response(
          JSON.stringify(
            acknowledgementFor(body, {
              issues: [
                {
                  code: "invalid_preference_reference",
                  entity: "active_workspace",
                  source_id: "ws_missing",
                },
                {
                  code: "invalid_preference_reference",
                  entity: "active_session",
                  source_id: "sess_missing",
                },
                {
                  code: "invalid_preference_reference",
                  entity: "provider_selection",
                  source_id: "prov_missing",
                },
              ],
            }),
          ),
          { status: 200 },
        );
      },
    );

    const result = await runM1BrowserMigration({
      storage,
      lock: immediateMigrationLock,
      fetch: fetchMock,
      idGenerator: () => "migration-issued-preferences",
      now: () => "2026-07-26T00:00:00.000Z",
    });

    expect(result.status).toBe("complete");
  });

  it("preserves every legacy key after a successful C0 migration", async () => {
    const initial = legacyState();
    const storage = new MemoryStorage(initial);

    const result = await runM1BrowserMigration({
      storage,
      lock: immediateMigrationLock,
      fetch: successfulFetch(),
      idGenerator: () => "migration-preserve",
      now: () => "2026-07-26T00:00:00.000Z",
    });

    expect(result.status).toBe("complete");
    for (const [key, value] of Object.entries(initial)) {
      expect(storage.peek(key)).toBe(value);
    }
    expect(storage.removes).toEqual([]);
  });
});
