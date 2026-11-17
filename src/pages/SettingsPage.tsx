import { useEffect, useMemo, useState } from "react";
import {
  Check,
  FolderOpen,
  Gauge,
  KeyRound,
  Plus,
  RefreshCw,
  Save,
  Settings2,
  SlidersHorizontal,
  TerminalSquare,
  Trash2,
  Undo2,
} from "lucide-react";
import type { DownloadManager } from "@/hooks/useDownloadManager";
import type { Engine, HardcoverStatus, Settings, ToolStatus } from "@/types";
import { ENGINES } from "@/types";
import { api, isTauriRuntime } from "@/lib/tauri";
import { bytesToGigabytes, gigabytesToBytes } from "@/lib/storage";
import { cn, formatBytes } from "@/lib/utils";
import { PageHeader } from "@/components/ui/PageHeader";

const EMPTY: Settings = {
  downloadDir: "",
  lncrawlPath: null,
  galleryDlPath: null,
  agetPath: null,
  defaultEngine: "webnovel",
  mangaFormat: "cbz",
  mangaThrottle: true,
  mangaLanguage: "en",
  bookLanguage: null,
  sourceLimits: [],
  maxLibraryBytes: 0,
  cleanupWorkAfterImport: true,
};

export function SettingsPage({ manager }: { manager: DownloadManager }) {
  const [form, setForm] = useState<Settings>(manager.settings ?? EMPTY);
  const [saving, setSaving] = useState(false);
  const [hardcover, setHardcover] = useState<HardcoverStatus | null>(null);
  const [hardcoverToken, setHardcoverToken] = useState("");
  const [hardcoverBusy, setHardcoverBusy] = useState(false);

  useEffect(() => {
    if (manager.settings) setForm(manager.settings);
  }, [manager.settings]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let disposed = false;
    api
      .getHardcoverStatus()
      .then((status) => {
        if (!disposed) setHardcover(status);
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
    };
  }, []);

  async function saveHardcoverToken() {
    const token = hardcoverToken.trim();
    if (token.length < 20) {
      manager.showNotice(
        "error",
        "Token too short",
        "Paste the full Hardcover API token.",
      );
      return;
    }
    setHardcoverBusy(true);
    try {
      const status = await api.setHardcoverToken(token);
      setHardcover(status);
      setHardcoverToken("");
      manager.showNotice(
        "success",
        "Hardcover token stored",
        "The token is written to a local file and used only for catalogue queries.",
      );
    } catch (error) {
      manager.showNotice(
        "error",
        "Could not store token",
        error instanceof Error ? error.message : String(error),
      );
    } finally {
      setHardcoverBusy(false);
    }
  }

  async function clearHardcoverToken() {
    setHardcoverBusy(true);
    try {
      const status = await api.clearHardcoverToken();
      setHardcover(status);
      setHardcoverToken("");
      manager.showNotice(
        "success",
        "Hardcover token removed",
        "Catalogue searches now use Open Library only.",
      );
    } catch (error) {
      manager.showNotice(
        "error",
        "Could not remove token",
        error instanceof Error ? error.message : String(error),
      );
    } finally {
      setHardcoverBusy(false);
    }
  }

  const dirty = useMemo(
    () =>
      manager.settings
        ? JSON.stringify(form) !== JSON.stringify(manager.settings)
        : false,
    [form, manager.settings],
  );

  const toolMap = useMemo(() => {
    const map = new Map<string, ToolStatus>();
    for (const tool of manager.toolsList) map.set(tool.id, tool);
    return map;
  }, [manager.toolsList]);

  function update<K extends keyof Settings>(key: K, value: Settings[K]) {
    setForm((current) => ({ ...current, [key]: value }));
  }

  function updateLimit(
    index: number,
    patch: Partial<Settings["sourceLimits"][number]>,
  ) {
    setForm((current) => ({
      ...current,
      sourceLimits: current.sourceLimits.map((limit, position) =>
        position === index ? { ...limit, ...patch } : limit,
      ),
    }));
  }

  async function save() {
    setSaving(true);
    await manager.saveSettings(form);
    setSaving(false);
  }

  return (
    <div className="space-y-4">
      <PageHeader
        eyebrow="Configuration"
        title="Settings"
        description="Point NovelForge at the toolchain and choose the defaults every new transfer starts from."
        actions={
          <>
            <button
              type="button"
              onClick={() => manager.settings && setForm(manager.settings)}
              disabled={!dirty}
              className="btn-secondary"
            >
              <Undo2 className="h-3 w-3" strokeWidth={1.8} />
              Revert
            </button>
            <button
              type="button"
              onClick={() => void save()}
              disabled={!dirty || saving}
              className="btn-primary h-9 px-4"
            >
              {saving ? (
                <RefreshCw className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
              ) : (
                <Save className="h-3.5 w-3.5" strokeWidth={1.9} />
              )}
              Save changes
            </button>
          </>
        }
      />

      <div className="grid grid-cols-1 gap-3.5 xl:grid-cols-12">
        <div className="space-y-3.5 xl:col-span-7">
          <section className="panel p-4">
            <PanelTitle
              icon={<FolderOpen className="h-3.5 w-3.5" strokeWidth={1.8} />}
              title="Storage"
              detail="All outputs are written under this root."
            />
            <div className="mt-3 space-y-3">
              <Field label="Download directory" hint="Absolute path or ~/…">
                <input
                  value={form.downloadDir}
                  onChange={(event) => update("downloadDir", event.target.value)}
                  spellCheck={false}
                  placeholder="~/Downloads/NovelForge"
                  className="field h-9 w-full px-3 font-mono text-[13px] text-parchment-200 outline-none"
                />
              </Field>
              <div className="flex items-center justify-between border border-parchment-100/8 bg-black/20 px-3 py-2.5">
                <div className="text-[11.5px] leading-relaxed text-parchment-500">
                  Reveal, verify and library scans are refused outside this sandbox.
                </div>
                <button
                  type="button"
                  onClick={() => void manager.openDownloads()}
                  className="btn-secondary h-7 shrink-0 px-2.5"
                >
                  <FolderOpen className="h-3 w-3" strokeWidth={1.8} />
                  Open
                </button>
              </div>

              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <Field label="Library quota" hint="0 = unlimited">
                  <QuotaField
                    value={form.maxLibraryBytes}
                    onChange={(value) => update("maxLibraryBytes", value)}
                  />
                </Field>
                <Field label="Current library" hint="measured on scan">
                  <div className="field flex h-9 items-center px-3 font-mono text-[13px] text-parchment-300">
                    {formatBytes(manager.library.stats.totalBytes)}
                  </div>
                </Field>
              </div>
              <p className="text-[11px] leading-relaxed text-parchment-600">
                A new download is refused once the library reaches this size.
                Updates and retries are never blocked, and nothing is deleted to
                enforce the quota.
              </p>

              <button
                type="button"
                onClick={() =>
                  update(
                    "cleanupWorkAfterImport",
                    !form.cleanupWorkAfterImport,
                  )
                }
                aria-pressed={form.cleanupWorkAfterImport}
                className="flex w-full items-center justify-between border border-parchment-100/9 bg-parchment-100/[0.018] px-3 py-2.5 text-left transition-colors hover:border-parchment-100/18"
              >
                <div>
                  <div className="text-[12.5px] font-medium text-parchment-200">
                    Remove engine work files after import
                  </div>
                  <div className="mt-0.5 text-[11px] text-parchment-600">
                    Deletes the per-job crawler tree once the EPUB is verified.
                    Disable only while debugging a source adapter.
                  </div>
                </div>
                <span className="toggle" data-on={form.cleanupWorkAfterImport} />
              </button>
            </div>
          </section>

          <section className="panel p-4">
            <PanelTitle
              icon={<SlidersHorizontal className="h-3.5 w-3.5" strokeWidth={1.8} />}
              title="Engine defaults"
              detail="Applied to new transfers; per-job options can still override."
            />
            <div className="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2">
              <Field label="Default engine">
                <SelectControl
                  value={form.defaultEngine}
                  onChange={(value) => update("defaultEngine", value as Engine)}
                  options={[
                    { value: "webnovel", label: ENGINES.webnovel.label },
                    { value: "manga", label: ENGINES.manga.label },
                    { value: "books", label: ENGINES.books.label },
                  ]}
                />
              </Field>
              <Field label="Manga container">
                <SelectControl
                  value={form.mangaFormat}
                  onChange={(value) =>
                    update("mangaFormat", value as Settings["mangaFormat"])
                  }
                  options={[
                    { value: "cbz", label: "CBZ · comic book zip" },
                    { value: "zip", label: "ZIP · plain archive" },
                  ]}
                />
              </Field>
              <Field label="Manga language">
                <input
                  value={form.mangaLanguage ?? ""}
                  onChange={(event) =>
                    update("mangaLanguage", event.target.value.trim() || null)
                  }
                  placeholder="all"
                  spellCheck={false}
                  className="field h-9 w-full px-3 font-mono text-[13px] text-parchment-200 outline-none"
                />
              </Field>
              <Field label="Book language">
                <input
                  value={form.bookLanguage ?? ""}
                  onChange={(event) =>
                    update("bookLanguage", event.target.value.trim() || null)
                  }
                  placeholder="any"
                  spellCheck={false}
                  className="field h-9 w-full px-3 font-mono text-[13px] text-parchment-200 outline-none"
                />
              </Field>
            </div>

            <button
              type="button"
              onClick={() => update("mangaThrottle", !form.mangaThrottle)}
              aria-pressed={form.mangaThrottle}
              className="mt-3 flex w-full items-center justify-between border border-parchment-100/9 bg-parchment-100/[0.018] px-3 py-2.5 text-left transition-colors hover:border-parchment-100/18"
            >
              <div>
                <div className="text-[12.5px] font-medium text-parchment-200">
                  Throttle manga requests
                </div>
                <div className="mt-0.5 text-[11px] text-parchment-600">
                  Pauses between HTTP requests; gallery-dl has no job pool.
                </div>
              </div>
              <span className="toggle" data-on={form.mangaThrottle} />
            </button>
          </section>

          <section className="panel p-4">
            <PanelTitle
              icon={<KeyRound className="h-3.5 w-3.5" strokeWidth={1.8} />}
              title="Book metadata · Hardcover"
              detail="Optional. A free personal token adds series positions, authors and ISBNs to catalogue search."
            />

            <div className="mt-3 border border-parchment-100/9 bg-parchment-100/[0.015] px-3 py-2.5">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  <span
                    className={cn(
                      "h-1.5 w-1.5 rounded-full",
                      hardcover?.configured ? "bg-jade-400" : "bg-parchment-600",
                    )}
                  />
                  <span className="text-[12px] font-medium text-parchment-200">
                    {hardcover?.configured ? "Token configured" : "No token configured"}
                  </span>
                </div>
                <span className="text-[10px] uppercase tracking-[0.1em] text-parchment-600">
                  {hardcover?.configured ? "Hardcover + Open Library" : "Open Library only"}
                </span>
              </div>
              <p className="mt-1.5 text-[10.5px] leading-relaxed text-parchment-600">
                Stored locally at{" "}
                <code className="text-parchment-400">
                  {hardcover?.path ?? "the app config folder"}
                </code>
                . It is never sent to NovelForge or any hosted server; the only
                destination is <code className="text-parchment-400">api.hardcover.app</code>.
              </p>
            </div>

            <div className="mt-3 flex flex-col gap-2 sm:flex-row">
              <input
                type="password"
                value={hardcoverToken}
                onChange={(event) => setHardcoverToken(event.target.value)}
                placeholder={
                  hardcover?.configured
                    ? "Paste a new token to replace the stored one"
                    : "Paste your Hardcover API token"
                }
                spellCheck={false}
                autoComplete="off"
                aria-label="Hardcover API token"
                className="field h-9 min-w-0 flex-1 px-3 font-mono text-[12px] text-parchment-200 outline-none"
              />
              <button
                type="button"
                onClick={() => void saveHardcoverToken()}
                disabled={hardcoverBusy || hardcoverToken.trim().length < 20}
                className="btn-secondary h-9 shrink-0 gap-1.5 px-3"
              >
                {hardcoverBusy ? (
                  <RefreshCw className="h-3 w-3 animate-spin" strokeWidth={2} />
                ) : (
                  <Save className="h-3 w-3" strokeWidth={1.9} />
                )}
                Save token
              </button>
              {hardcover?.configured && (
                <button
                  type="button"
                  onClick={() => void clearHardcoverToken()}
                  disabled={hardcoverBusy}
                  className="btn-secondary h-9 shrink-0 gap-1.5 px-3"
                >
                  <Trash2 className="h-3 w-3" strokeWidth={1.9} />
                  Remove
                </button>
              )}
            </div>
            <p className="mt-2 text-[10px] leading-relaxed text-parchment-600">
              Create a free personal token in Hardcover → Settings → API. The
              free tier allows 5,000 requests per day (60/minute); NovelForge
              caches catalogue results for one hour.
            </p>
          </section>

          <section className="panel p-4">
            <PanelTitle
              icon={<Gauge className="h-3.5 w-3.5" strokeWidth={1.8} />}
              title="Source transport"
              detail="Per-host concurrency, request spacing and lncrawl chapter rate. Built-in profiles are shown in Guardrails."
            />
            <div className="mt-3 space-y-2">
              {form.sourceLimits.length === 0 && (
                <div className="border border-dashed border-parchment-100/10 px-3 py-3 text-[11.5px] leading-relaxed text-parchment-500">
                  No overrides. Each source uses its built-in safe profile and
                  ramps only after a clean streak.
                </div>
              )}
              {form.sourceLimits.map((limit, index) => (
                <div
                  key={index}
                  className="grid grid-cols-[minmax(0,1fr)_58px_70px_58px_26px] items-end gap-1.5 border border-parchment-100/[0.055] bg-parchment-100/[0.015] p-2"
                >
                  <label className="min-w-0">
                    <span className="mb-1 block font-mono text-[9.5px] uppercase tracking-[0.12em] text-parchment-600">
                      host
                    </span>
                    <input
                      value={limit.host}
                      onChange={(event) =>
                        updateLimit(index, { host: event.target.value })
                      }
                      placeholder="novelfull.net"
                      spellCheck={false}
                      className="field h-8 w-full px-2 font-mono text-[12px] text-parchment-200 outline-none"
                    />
                  </label>
                  <label>
                    <span className="mb-1 block font-mono text-[9.5px] uppercase tracking-[0.12em] text-parchment-600">
                      conc
                    </span>
                    <select
                      value={String(limit.concurrency)}
                      onChange={(event) =>
                        updateLimit(index, {
                          concurrency: Number(event.target.value),
                        })
                      }
                      className="field h-8 w-full px-1 font-mono text-[12px] text-parchment-200 outline-none"
                    >
                      {[1, 2, 3, 4, 5, 6, 7, 8].map((value) => (
                        <option key={value} value={value}>
                          {value}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    <span className="mb-1 block font-mono text-[9.5px] uppercase tracking-[0.12em] text-parchment-600">
                      gap ms
                    </span>
                    <input
                      type="number"
                      min={0}
                      max={10000}
                      step={100}
                      value={limit.requestGapMs}
                      onChange={(event) =>
                        updateLimit(index, {
                          requestGapMs: Number(event.target.value),
                        })
                      }
                      className="field h-8 w-full px-2 font-mono text-[12px] text-parchment-200 outline-none"
                    />
                  </label>
                  <label>
                    <span className="mb-1 block font-mono text-[9.5px] uppercase tracking-[0.12em] text-parchment-600">
                      req/s
                    </span>
                    <input
                      type="number"
                      min={0.1}
                      max={20}
                      step={0.1}
                      value={limit.chapterRate}
                      onChange={(event) =>
                        updateLimit(index, {
                          chapterRate: Number(event.target.value),
                        })
                      }
                      className="field h-8 w-full px-2 font-mono text-[12px] text-parchment-200 outline-none"
                    />
                  </label>
                  <button
                    type="button"
                    onClick={() =>
                      update(
                        "sourceLimits",
                        form.sourceLimits.filter(
                          (_, position) => position !== index,
                        ),
                      )
                    }
                    className="icon-button h-8 w-8"
                    aria-label="Remove source override"
                    title="Remove source override"
                  >
                    <Trash2 className="h-3 w-3" strokeWidth={1.8} />
                  </button>
                </div>
              ))}
              <button
                type="button"
                onClick={() =>
                  update("sourceLimits", [
                    ...form.sourceLimits,
                    {
                      host: "",
                      concurrency: 1,
                      requestGapMs: 900,
                      chapterRate: 2,
                    },
                  ])
                }
                className="btn-secondary h-7 w-full justify-center"
              >
                <Plus className="h-3 w-3" strokeWidth={1.9} />
                Add host override
              </button>
            </div>
          </section>
        </div>

        <section className="panel flex flex-col p-4 xl:col-span-5">
          <div className="flex items-start justify-between gap-3">
            <PanelTitle
              icon={<TerminalSquare className="h-3.5 w-3.5" strokeWidth={1.8} />}
              title="Toolchain paths"
              detail="Leave blank to auto-discover on PATH."
            />
            <button
              type="button"
              onClick={() => void manager.refreshTools()}
              className="btn-secondary h-7 shrink-0 px-2.5"
            >
              <RefreshCw className="h-3 w-3" strokeWidth={1.8} />
              Re-check
            </button>
          </div>

          <div className="mt-3 space-y-3">
            <ToolPathField
              label="lncrawl binary"
              placeholder="~/.local/bin/lncrawl"
              value={form.lncrawlPath}
              tool={toolMap.get("lncrawl")}
              onChange={(value) => update("lncrawlPath", value)}
            />
            <ToolPathField
              label="gallery-dl binary"
              placeholder="~/.local/bin/gallery-dl"
              value={form.galleryDlPath}
              tool={toolMap.get("gallery_dl")}
              onChange={(value) => update("galleryDlPath", value)}
            />
            <ToolPathField
              label="aget wrapper"
              placeholder="~/.local/bin/aget"
              value={form.agetPath}
              tool={toolMap.get("aget")}
              onChange={(value) => update("agetPath", value)}
            />
          </div>

          <div className="mt-auto flex items-start gap-2.5 border border-parchment-100/8 bg-black/20 px-3 py-2.5 pt-2.5">
            <Settings2 className="mt-[1px] h-3.5 w-3.5 shrink-0 text-parchment-600" strokeWidth={1.8} />
            <p className="text-[11.5px] leading-relaxed text-parchment-500">
              Changes are written to{" "}
              <span className="font-mono text-[11px] text-parchment-400">
                ~/.config/com.novelforge.app/settings.json
              </span>{" "}
              and validated before they are accepted.
            </p>
          </div>

          {dirty ? (
            <div className="mt-3 flex items-center gap-2 border border-gold-400/20 bg-gold-400/[0.055] px-2.5 py-1.5 text-[11px] text-gold-300">
              <span className="h-1 w-1 bg-gold-300" />
              Unsaved changes
            </div>
          ) : manager.settings ? (
            <div className="mt-3 flex items-center gap-2 border border-jade-400/15 bg-jade-400/[0.045] px-2.5 py-1.5 text-[11px] text-jade-300">
              <Check className="h-3 w-3" strokeWidth={2.2} />
              Configuration synced
            </div>
          ) : null}
        </section>
      </div>
    </div>
  );
}

function PanelTitle({
  icon,
  title,
  detail,
}: {
  icon: React.ReactNode;
  title: string;
  detail: string;
}) {
  return (
    <div className="flex items-start gap-2">
      <span className="mt-[1px] text-vermilion-300/80">{icon}</span>
      <div>
        <div className="text-[13.5px] font-semibold text-parchment-200">{title}</div>
        <div className="mt-0.5 text-[11.5px] text-parchment-600">{detail}</div>
      </div>
    </div>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="block">
      <div className="mb-1.5 flex items-center justify-between">
        <span className="text-[10px] font-semibold uppercase tracking-[0.14em] text-parchment-500">
          {label}
        </span>
        {hint && <span className="text-[10.5px] text-parchment-700">{hint}</span>}
      </div>
      {children}
    </label>
  );
}

function QuotaField({
  value,
  onChange,
}: {
  value: number;
  onChange: (value: number) => void;
}) {
  const [draft, setDraft] = useState(String(bytesToGigabytes(value)));

  useEffect(() => {
    setDraft(String(bytesToGigabytes(value)));
  }, [value]);

  function commit() {
    const parsed = Number(draft.trim().replace(",", "."));
    const bytes = Number.isFinite(parsed) ? gigabytesToBytes(parsed) : 0;
    setDraft(String(bytesToGigabytes(bytes)));
    onChange(bytes);
  }

  return (
    <div className="field flex h-9 items-center gap-2 px-3">
      <input
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            commit();
          }
        }}
        inputMode="decimal"
        spellCheck={false}
        aria-label="Library quota in gigabytes"
        className="min-w-0 flex-1 font-mono text-[13px] text-parchment-200 outline-none"
      />
      <span className="shrink-0 font-mono text-[11px] uppercase tracking-[0.1em] text-parchment-600">
        GB
      </span>
    </div>
  );
}

function ToolPathField({
  label,
  placeholder,
  value,
  tool,
  onChange,
}: {
  label: string;
  placeholder: string;
  value: string | null;
  tool: ToolStatus | undefined;
  onChange: (value: string | null) => void;
}) {
  return (
    <div>
      <div className="mb-1.5 flex items-center justify-between gap-2">
        <span className="text-[10px] font-semibold uppercase tracking-[0.14em] text-parchment-500">
          {label}
        </span>
        <span className="flex items-center gap-1.5 text-[10.5px] text-parchment-600">
          <span
            className={cn(
              "h-1.5 w-1.5",
              tool?.found ? "bg-jade-400/90" : "bg-vermilion-500/80",
            )}
          />
          {tool?.found ? "detected" : "not found"}
        </span>
      </div>
      <input
        value={value ?? ""}
        onChange={(event) => onChange(event.target.value.trim() || null)}
        placeholder={placeholder}
        spellCheck={false}
        className="field h-9 w-full px-3 font-mono text-[12.5px] text-parchment-200 outline-none"
      />
      <div className="mt-1 truncate text-[10.5px] text-parchment-700" title={tool?.path ?? undefined}>
        {tool?.version ?? tool?.detail ?? "No version reported"}
      </div>
    </div>
  );
}

function SelectControl({
  value,
  onChange,
  options,
}: {
  value: string;
  onChange: (value: string) => void;
  options: Array<{ value: string; label: string }>;
}) {
  return (
    <select
      value={value}
      onChange={(event) => onChange(event.target.value)}
      className="select-control w-full px-2.5"
    >
      {options.map((option) => (
        <option key={option.value} value={option.value} className="bg-[#17171a]">
          {option.label}
        </option>
      ))}
    </select>
  );
}
