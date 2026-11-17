import { useEffect, useState } from "react";
import {
  AlertTriangle,
  BookMarked,
  Check,
  Copy,
  FolderLock,
  Gauge,
  Library,
  PackageCheck,
  RefreshCw,
  ShieldCheck,
  TerminalSquare,
} from "lucide-react";
import type { DownloadManager } from "@/hooks/useDownloadManager";
import type {
  SourceProfileSummary,
  ToolStatus,
  TransportReport,
} from "@/types";
import { api } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import { PageHeader } from "@/components/ui/PageHeader";

const RULES = [
  {
    icon: PackageCheck,
    title: "Real container, real MIME",
    body: "EPUB outputs must open as a ZIP and carry a mimetype entry equal to application/epub+zip. No exceptions.",
  },
  {
    icon: ShieldCheck,
    title: "OCF ordering for books",
    body: "Books require mimetype to be the first entry and stored uncompressed. Webnovel EPUBs may warn but still pass.",
  },
  {
    icon: AlertTriangle,
    title: "Delete on rejection",
    body: "A container that fails validation is deleted immediately and the job is marked rejected — nothing invalid reaches the library.",
  },
  {
    icon: FolderLock,
    title: "Path sandbox",
    body: "Reveal, verify and library scans are canonicalized and refused unless they resolve inside the download root.",
  },
  {
    icon: Library,
    title: "Public-domain sources are opt-in",
    body: "Project Gutenberg and Standard Ebooks are lawful catalogues of older, public-domain works. They never join the Auto order; selecting one only changes the source used for the next Fetch.",
  },
  {
    icon: BookMarked,
    title: "Manga availability is licensing, not failure",
    body: "MangaDex can only host chapters its scan groups may publish. A licensed chapter is reported by number, with the official publisher link, and is never scraped from that publisher page — choose a different extractor if the series is available elsewhere.",
  },
] as const;

const POLICIES = [
  "Every child process runs in its own group with stdin closed, so a downloader can never block on a prompt.",
  "Cancellation sends SIGTERM to the group, then SIGKILL after a grace period. Pause uses SIGSTOP and resume SIGCONT.",
  "Arguments are passed as vectors — no shell interpolation, no command injection through pasted URLs.",
  "Input is length-checked and control characters are stripped before a job is created.",
  "Terminal output is ANSI-stripped and bounded, keeping log events safe to render.",
] as const;

export function GuardrailsPage({ manager }: { manager: DownloadManager }) {
  const [checking, setChecking] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);
  const [transport, setTransport] = useState<TransportReport | null>(null);
  const [profiles, setProfiles] = useState<SourceProfileSummary[]>([]);

  async function loadTransport() {
    try {
      setTransport(await api.getTransportReport());
    } catch {
      setTransport(null);
    }
  }

  async function loadProfiles() {
    try {
      setProfiles(await api.listSourceProfiles());
    } catch {
      setProfiles([]);
    }
  }

  useEffect(() => {
    void loadTransport();
    void loadProfiles();
  }, []);

  async function recheck() {
    setChecking(true);
    const report = await manager.refreshTools();
    await loadTransport();
    await loadProfiles();
    setChecking(false);
    if (report) {
      manager.showNotice(
        report.allReady ? "success" : "error",
        report.allReady ? "Toolchain ready" : "Toolchain incomplete",
        report.allReady
          ? "All four binaries responded to a version probe."
          : "One or more engines are missing. Review the hints below.",
      );
    }
  }

  async function copyHint(tool: ToolStatus) {
    try {
      await navigator.clipboard.writeText(tool.hint);
      setCopied(tool.id);
      window.setTimeout(() => setCopied(null), 1600);
    } catch {
      manager.showNotice("error", "Clipboard unavailable", "Select the command and copy it manually.");
    }
  }

  return (
    <div className="space-y-4">
      <PageHeader
        eyebrow="Integrity policy"
        title="Guardrails"
        description="Verification is enforced in Rust, independent of what the downloader claims. The UI only ever reflects a container that passed inspection."
        actions={
          <button
            type="button"
            onClick={() => void recheck()}
            disabled={checking}
            className="btn-secondary"
          >
            <RefreshCw
              className={cn("h-3 w-3", checking && "animate-spin")}
              strokeWidth={1.8}
            />
            Run tool check
          </button>
        }
      />

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4">
        {RULES.map((rule) => {
          const Icon = rule.icon;
          return (
            <div key={rule.title} className="panel p-4">
              <div className="flex h-8 w-8 items-center justify-center border border-vermilion-500/25 bg-vermilion-500/[0.07]">
                <Icon className="h-3.5 w-3.5 text-vermilion-300" strokeWidth={1.7} />
              </div>
              <div className="mt-2.5 text-[13px] font-semibold text-parchment-200">
                {rule.title}
              </div>
              <p className="mt-1 text-[11.5px] leading-relaxed text-parchment-500">
                {rule.body}
              </p>
            </div>
          );
        })}
      </div>

      <div className="grid grid-cols-1 gap-3.5 xl:grid-cols-12">
        <section className="panel p-4 xl:col-span-7">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <TerminalSquare className="h-3.5 w-3.5 text-vermilion-300/80" strokeWidth={1.8} />
              <span className="text-[13px] font-semibold text-parchment-200">
                Engine toolchain
              </span>
            </div>
            <span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-parchment-600">
              {manager.toolsList.filter((tool) => tool.found).length}/
              {manager.toolsList.length || 3} online
            </span>
          </div>

          <div className="mt-3 space-y-1">
            {manager.toolsList.length === 0 && (
              <div className="border border-dashed border-parchment-100/10 px-4 py-6 text-center text-[12.5px] text-parchment-500">
                No probe results yet. Run a tool check.
              </div>
            )}
            {manager.toolsList.map((tool) => (
              <div
                key={tool.id}
                className="border border-parchment-100/[0.055] bg-parchment-100/[0.015] px-3 py-2.5"
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="flex min-w-0 items-start gap-2.5">
                    <span
                      className={cn(
                        "mt-[5px] h-1.5 w-1.5 shrink-0",
                        tool.found ? "bg-jade-400" : "bg-vermilion-500",
                      )}
                    />
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-[12.5px] font-medium text-parchment-200">
                          {tool.label}
                        </span>
                        <span className="chip h-[20px] px-1.5 text-[10px]">
                          {tool.id}
                        </span>
                      </div>
                      <div
                        className="mt-1 truncate font-mono text-[11px] text-parchment-600"
                        title={tool.path ?? undefined}
                      >
                        {tool.path ?? "not resolved"}
                      </div>
                      {tool.version && (
                        <div className="mt-0.5 truncate font-mono text-[10.5px] text-parchment-700">
                          {tool.version}
                        </div>
                      )}
                      {!tool.found && tool.hint && (
                        <div className="mt-1.5 flex items-start gap-1.5">
                          <code className="min-w-0 flex-1 select-all break-all border border-parchment-100/8 bg-black/30 px-1.5 py-1 font-mono text-[10.5px] leading-relaxed text-gold-300/80">
                            {tool.hint}
                          </code>
                          <button
                            type="button"
                            onClick={() => void copyHint(tool)}
                            className="icon-button h-6 w-6 shrink-0"
                            aria-label="Copy install command"
                            title="Copy install command"
                          >
                            {copied === tool.id ? (
                              <Check className="h-3 w-3 text-jade-300" strokeWidth={2.2} />
                            ) : (
                              <Copy className="h-3 w-3" strokeWidth={1.8} />
                            )}
                          </button>
                        </div>
                      )}
                      {tool.found && tool.detail && (
                        <div className="mt-1 text-[11px] text-parchment-600">
                          {tool.detail}
                        </div>
                      )}
                    </div>
                  </div>
                  <span
                    className={cn(
                      "shrink-0 border px-1.5 py-[2px] text-[9.5px] font-semibold uppercase tracking-[0.12em]",
                      tool.found
                        ? "border-jade-400/20 bg-jade-400/[0.06] text-jade-300"
                        : "border-vermilion-500/25 bg-vermilion-500/[0.07] text-vermilion-200",
                    )}
                  >
                    {tool.found ? "ready" : "missing"}
                  </span>
                </div>
              </div>
            ))}
          </div>
        </section>

        <section className="panel p-4 xl:col-span-5">
          <div className="flex items-center gap-2">
            <ShieldCheck className="h-3.5 w-3.5 text-vermilion-300/80" strokeWidth={1.8} />
            <span className="text-[13px] font-semibold text-parchment-200">
              Runtime controls
            </span>
          </div>
          <ul className="mt-3 space-y-2.5">
            {POLICIES.map((policy) => (
              <li key={policy} className="flex items-start gap-2.5">
                <span className="mt-[5px] h-1 w-1 shrink-0 bg-vermilion-400/70" />
                <span className="text-[12px] leading-relaxed text-parchment-500">
                  {policy}
                </span>
              </li>
            ))}
          </ul>

          <div className="mt-4 border border-vermilion-500/18 bg-vermilion-500/[0.04] p-3">
            <div className="flex items-center gap-2 text-[11.5px] font-semibold uppercase tracking-[0.13em] text-vermilion-200">
              <AlertTriangle className="h-3 w-3" strokeWidth={2} />
              Fail closed
            </div>
            <p className="mt-1.5 text-[11.5px] leading-relaxed text-parchment-500">
              If a verifier cannot read a file, the download fails rather than
              reporting success. Unknown extensions must be non-empty and are
              never treated as a valid EPUB.
            </p>
          </div>
        </section>
      </div>

      <section className="panel p-4">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <Gauge className="h-3.5 w-3.5 text-vermilion-300/80" strokeWidth={1.8} />
            <span className="text-[13px] font-semibold text-parchment-200">
              Source transport
            </span>
          </div>
          <button
            type="button"
            onClick={() => void loadTransport()}
            className="btn-secondary h-7 px-2.5"
          >
            <RefreshCw className="h-3 w-3" strokeWidth={1.8} />
            Refresh
          </button>
        </div>
        <p className="mt-1.5 text-[11.5px] leading-relaxed text-parchment-500">
          Per-host gates start at the source profile&apos;s tested concurrency,
          halve immediately on 429/503, and only ramp after 20 clean responses.
          Blocked hosts cool down instead of retrying in a loop.
        </p>

        {!transport || transport.hosts.length === 0 ? (
          <div className="mt-3 border border-dashed border-parchment-100/10 px-3 py-5 text-center text-[11.5px] text-parchment-500">
            No source requests this session. Fetch metadata or a chapter index
            to populate the per-host report.
          </div>
        ) : (
          <div className="mt-3 overflow-x-auto">
            <table className="w-full min-w-[760px] border-collapse text-left">
              <thead>
                <tr className="border-b border-parchment-100/8 font-mono text-[9.5px] uppercase tracking-[0.12em] text-parchment-600">
                  <th className="py-1.5 pr-3 font-normal">host / profile</th>
                  <th className="px-2 py-1.5 text-right font-normal">conc</th>
                  <th className="px-2 py-1.5 text-right font-normal">gap</th>
                  <th className="px-2 py-1.5 text-right font-normal">req/s</th>
                  <th className="px-2 py-1.5 text-right font-normal">requests</th>
                  <th className="px-2 py-1.5 text-right font-normal">retries</th>
                  <th className="px-2 py-1.5 text-right font-normal">429</th>
                  <th className="px-2 py-1.5 text-right font-normal">curl</th>
                  <th className="px-2 py-1.5 text-right font-normal">avg</th>
                </tr>
              </thead>
              <tbody>
                {transport.hosts.map((host) => (
                  <tr
                    key={host.host}
                    className="border-b border-parchment-100/[0.045] align-top"
                  >
                    <td className="py-2 pr-3">
                      <div className="font-mono text-[11.5px] text-parchment-200">
                        {host.host}
                      </div>
                      <div className="mt-0.5 text-[10.5px] text-parchment-500">
                        {host.label} · {host.profile}
                      </div>
                      <div className="mt-1 max-w-[360px] text-[10px] leading-relaxed text-parchment-600">
                        {host.note}
                      </div>
                    </td>
                    <td className="px-2 py-2 text-right font-mono text-[11px] tabular-nums text-parchment-300">
                      {host.currentConcurrency}/{host.maxConcurrency}
                    </td>
                    <td className="px-2 py-2 text-right font-mono text-[11px] tabular-nums text-parchment-400">
                      {host.requestGapMs} ms
                    </td>
                    <td className="px-2 py-2 text-right font-mono text-[11px] tabular-nums text-parchment-400">
                      {host.chapterRate}
                    </td>
                    <td className="px-2 py-2 text-right font-mono text-[11px] tabular-nums text-parchment-300">
                      {host.requests}
                    </td>
                    <td className="px-2 py-2 text-right font-mono text-[11px] tabular-nums text-parchment-400">
                      {host.retries}
                    </td>
                    <td
                      className={cn(
                        "px-2 py-2 text-right font-mono text-[11px] tabular-nums",
                        host.status429 > 0 ? "text-gold-300" : "text-parchment-500",
                      )}
                    >
                      {host.status429}
                    </td>
                    <td className="px-2 py-2 text-right font-mono text-[11px] tabular-nums text-parchment-400">
                      {host.curlRequests}
                    </td>
                    <td className="px-2 py-2 text-right font-mono text-[11px] tabular-nums text-parchment-400">
                      {host.avgLatencyMs} ms
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      <section className="panel p-4">
        <div className="flex items-center gap-2">
          <ShieldCheck
            className="h-3.5 w-3.5 text-vermilion-300/80"
            strokeWidth={1.8}
          />
          <span className="text-[13px] font-semibold text-parchment-200">
            Source support
          </span>
          <span className="ml-auto font-mono text-[10px] uppercase tracking-[0.12em] text-parchment-600">
            {profiles.filter((profile) => profile.downloadReady).length}/
            {profiles.length} download-ready
          </span>
        </div>
        <p className="mt-1.5 text-[11.5px] leading-relaxed text-parchment-500">
          Download-ready means search, a complete chapter index, a 1–3 chapter
          export and a truncated-TOC failure test all passed live. Everything
          else is shown at the level that was actually proven.
        </p>

        {profiles.length === 0 ? (
          <div className="mt-3 border border-dashed border-parchment-100/10 px-3 py-5 text-center text-[11.5px] text-parchment-500">
            Source registry unavailable.
          </div>
        ) : (
          <div className="mt-3 overflow-x-auto">
            <table className="w-full min-w-[680px] border-collapse text-left">
              <thead>
                <tr className="border-b border-parchment-100/8 font-mono text-[9.5px] uppercase tracking-[0.12em] text-parchment-600">
                  <th className="py-1.5 pr-3 font-normal">source</th>
                  <th className="px-2 py-1.5 font-normal">hosts</th>
                  <th className="px-2 py-1.5 font-normal">support</th>
                  <th className="px-2 py-1.5 text-center font-normal">search</th>
                  <th className="px-2 py-1.5 text-center font-normal">index</th>
                  <th className="px-2 py-1.5 font-normal">evidence</th>
                </tr>
              </thead>
              <tbody>
                {profiles.map((profile) => (
                  <tr
                    key={profile.id}
                    className="border-b border-parchment-100/[0.045] align-top"
                  >
                    <td className="py-2 pr-3">
                      <div className="text-[12px] font-medium text-parchment-200">
                        {profile.displayName}
                      </div>
                      <div className="mt-0.5 font-mono text-[10px] text-parchment-600">
                        {profile.id} · {profile.language}
                        {profile.originLanguages.length > 0 &&
                          ` · from ${profile.originLanguages.join("/")}`}
                      </div>
                    </td>
                    <td className="px-2 py-2 font-mono text-[10.5px] text-parchment-400">
                      {profile.hosts.join(", ")}
                    </td>
                    <td className="px-2 py-2">
                      <span
                        className={cn(
                          "border px-1.5 py-0.5 font-mono text-[9.5px] uppercase tracking-[0.1em]",
                          profile.downloadReady
                            ? "border-emerald-400/25 bg-emerald-400/[0.06] text-emerald-300"
                            : profile.support === "index-verified"
                              ? "border-gold-400/25 bg-gold-400/[0.05] text-gold-300"
                              : "border-parchment-100/12 text-parchment-500",
                        )}
                      >
                        {profile.supportLabel}
                      </span>
                    </td>
                    <td className="px-2 py-2 text-center font-mono text-[11px] text-parchment-400">
                      {profile.search ? "yes" : "—"}
                    </td>
                    <td className="px-2 py-2 text-center font-mono text-[11px] text-parchment-400">
                      {profile.index ? "yes" : "—"}
                    </td>
                    <td className="max-w-[320px] px-2 py-2 text-[10.5px] leading-relaxed text-parchment-500">
                      {profile.evidence}
                      {profile.availabilityNote && (
                        <div className="mt-1 border-l border-amber-500/40 pl-2 text-amber-200/80">
                          {profile.availabilityNote}
                        </div>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </div>
  );
}
