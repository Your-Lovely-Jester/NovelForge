import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";
import type { TaskPhase, TaskStatus } from "@/types";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function formatBytes(bytes: number, decimals = 1): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const exponent = Math.min(
    Math.floor(Math.log(bytes) / Math.log(1024)),
    units.length - 1,
  );
  const value = bytes / 1024 ** exponent;
  return `${value.toFixed(exponent === 0 ? 0 : decimals)} ${units[exponent]}`;
}

export function formatSpeed(bytesPerSecond: number): string {
  if (!Number.isFinite(bytesPerSecond) || bytesPerSecond < 1) return "—";
  return `${formatBytes(bytesPerSecond)}/s`;
}

/** Chapters per minute, rounded for a compact library row. */
export function formatPace(chaptersPerMinute: number): string {
  if (!Number.isFinite(chaptersPerMinute) || chaptersPerMinute <= 0) return "—";
  return chaptersPerMinute >= 100
    ? `${Math.round(chaptersPerMinute)} ch/min`
    : `${chaptersPerMinute.toFixed(1)} ch/min`;
}

export function formatDuration(totalSeconds: number): string {
  if (!Number.isFinite(totalSeconds) || totalSeconds < 0) return "00:00";
  const seconds = Math.floor(totalSeconds % 60);
  const minutes = Math.floor((totalSeconds / 60) % 60);
  const hours = Math.floor(totalSeconds / 3600);
  const pad = (value: number) => value.toString().padStart(2, "0");
  return hours > 0
    ? `${pad(hours)}:${pad(minutes)}:${pad(seconds)}`
    : `${pad(minutes)}:${pad(seconds)}`;
}

export function formatClock(timestamp: number): string {
  if (!timestamp) return "--:--:--";
  return new Date(timestamp).toLocaleTimeString([], {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function formatDate(timestamp: number): string {
  if (!timestamp) return "—";
  return new Date(timestamp * 1000).toLocaleDateString([], {
    year: "numeric",
    month: "short",
    day: "2-digit",
  });
}

const PHASE_LABELS: Record<TaskPhase, string> = {
  resolving: "Resolving source",
  fetchingDetails: "Fetching details",
  downloadingChapters: "Downloading chapters",
  downloadingImages: "Downloading images",
  packing: "Packing EPUB",
  importing: "Importing output",
  fetchingIndex: "Fetching full index",
  auditing: "Auditing chapters",
  retrying: "Retrying chapters",
  exporting: "Exporting structured files",
  verifying: "Verifying output",
  completed: "Complete",
};

export function phaseLabel(phase: TaskPhase | null | undefined): string {
  return phase ? PHASE_LABELS[phase] ?? "Working" : "Working";
}

export function isActive(status: TaskStatus): boolean {
  return status === "running" || status === "paused" || status === "verifying";
}

export function isTerminal(status: TaskStatus): boolean {
  return (
    status === "completed" ||
    status === "failed" ||
    status === "cancelled" ||
    status === "interrupted"
  );
}

export function pluralize(count: number, singular: string, plural = `${singular}s`) {
  return `${count} ${count === 1 ? singular : plural}`;
}

/** Host label for a stored source URL, without the leading www. */
export function hostFromUrl(value: string): string {
  try {
    return new URL(value).hostname.replace(/^www\./, "");
  } catch {
    return "local";
  }
}
