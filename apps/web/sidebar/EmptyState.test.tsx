import { Children, isValidElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { EmptyState } from "./EmptyState";

// Like ui-skin.test.tsx, exercise lifecycle explicitly in the Node environment
// without adding a DOM dependency. Re-renders retain state and ref slots.
const hooks = vi.hoisted(() => ({
  values: [] as unknown[],
  index: 0,
  generation: { current: 0 },
  cleanup: undefined as (() => void) | undefined,
  writes: vi.fn(),
  desktop: false,
  pick: vi.fn<() => Promise<unknown>>(),
}));
vi.mock("react", async (original) => ({
  ...await original<typeof import("react")>(),
  useMemo: (factory: () => unknown) => factory(),
  useRef: () => hooks.generation,
  useEffect: (effect: () => () => void) => { hooks.cleanup ??= effect(); },
  useState: (initial: unknown) => {
    const index = hooks.index++;
    if (!(index in hooks.values)) hooks.values[index] = initial;
    return [hooks.values[index], (value: unknown) => {
      hooks.writes(value);
      hooks.values[index] = value;
    }];
  },
}));
vi.mock("../copy/CopyProvider", () => ({ useCopy: () => ({ t: (key: string) => key }) }));
vi.mock("../product/product-client", () => ({
  createProductApiClient: () => ({ pickWorkspaceFolder: hooks.pick }),
}));
vi.mock("../platform/desktop-commands", () => ({
  desktopWorkspacePickerAvailable: () => hooks.desktop,
  selectDesktopWorkspace: () => hooks.pick(),
}));

type Control = {
  children?: ReactNode;
  id?: string;
  disabled?: boolean;
  value?: string;
  onClick?: () => void;
  onChange?: (event: { target: { value: string } }) => void;
  onSubmit?: (event: { preventDefault: () => void }) => void;
};
function controls(node: ReactNode): Control[] {
  return Children.toArray(node).flatMap((child) => isValidElement<Control>(child)
    ? [child.props, ...controls(child.props.children)] : []);
}
function mount() {
  const props = { recents: [], onOpenWorkspace: vi.fn(), onOpenRecent: vi.fn(), onOpenProviders: vi.fn() };
  function render() {
    hooks.index = 0;
    return controls(EmptyState(props));
  }
  const all = () => render();
  return {
    props,
    browse: () => all().find((control) => control.disabled !== undefined)!,
    input: () => all().find((control) => control.id === "empty-workspace-path")!,
    kind: () => all().find((control) => control.id === "empty-workspace-kind")!,
    submit: () => all().find((control) => control.onSubmit)!.onSubmit!({ preventDefault: vi.fn() }),
    all,
  };
}
function deferred() {
  let resolve!: (value: unknown) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<unknown>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
async function settle() {
  await Promise.resolve();
  await Promise.resolve();
}

beforeEach(() => {
  hooks.values = [];
  hooks.index = 0;
  hooks.generation = { current: 0 };
  hooks.cleanup = undefined;
  hooks.writes.mockClear();
  hooks.pick.mockReset();
});

describe.each([false, true])("EmptyState desktop=%s", (desktop) => {
  beforeEach(() => { hooks.desktop = desktop; });
  const selected = (path: string) => desktop ? path : { status: "selected", path };

  it("has one Browse control, populates only, and opens on explicit submit", async () => {
    hooks.pick.mockResolvedValue(selected("D:\\chosen"));
    const view = mount();
    expect(view.all().filter((control) => control.disabled !== undefined)).toHaveLength(1);
    view.browse().onClick!();
    await settle();
    expect(view.input().value).toBe("D:\\chosen");
    expect(view.props.onOpenWorkspace).not.toHaveBeenCalled();
    view.submit();
    expect(view.props.onOpenWorkspace).toHaveBeenCalledWith("D:\\chosen", "folder");
  });

  it.each(["path", "kind", "submit"])("ignores selection superseded by %s interaction", async (interaction) => {
    const pending = deferred();
    hooks.pick.mockReturnValue(pending.promise);
    const view = mount();
    view.input().onChange!({ target: { value: "D:\\manual" } });
    view.browse().onClick!();
    if (interaction === "path") view.input().onChange!({ target: { value: "D:\\manual" } });
    if (interaction === "kind") view.kind().onChange!({ target: { value: "repo" } });
    if (interaction === "submit") view.submit();
    hooks.writes.mockClear();
    pending.resolve(selected("D:\\stale"));
    await settle();
    expect(hooks.writes).not.toHaveBeenCalled();
    expect(view.input().value).toBe("D:\\manual");
    expect(view.props.onOpenWorkspace).toHaveBeenCalledTimes(interaction === "submit" ? 1 : 0);
  });

  it.each(["selected", "failure"])("ignores %s completion and finally after unmount", async (outcome) => {
    const pending = deferred();
    hooks.pick.mockReturnValue(pending.promise);
    const view = mount();
    view.browse().onClick!();
    hooks.cleanup!();
    hooks.writes.mockClear();
    if (outcome === "failure") pending.reject(new Error("stale error"));
    else pending.resolve(selected("D:\\stale"));
    await settle();
    expect(hooks.writes).not.toHaveBeenCalled();
    expect(view.props.onOpenWorkspace).not.toHaveBeenCalled();
  });

  it("does not let an older failure clear a newer picker's busy state", async () => {
    const old = deferred();
    const current = deferred();
    hooks.pick.mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise);
    const view = mount();
    view.browse().onClick!();
    view.input().onChange!({ target: { value: "D:\\manual" } });
    view.browse().onClick!();
    hooks.writes.mockClear();
    old.reject(new Error("stale error"));
    await settle();
    expect(hooks.writes).not.toHaveBeenCalled();
    expect(view.browse().disabled).toBe(true);
    current.resolve(selected("D:\\new"));
    await settle();
    expect(view.input().value).toBe("D:\\new");
    expect(view.browse().disabled).toBe(false);
    expect(view.props.onOpenWorkspace).not.toHaveBeenCalled();
  });
});
