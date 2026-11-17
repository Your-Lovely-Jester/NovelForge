import { useCallback, useEffect, useState } from "react";
import type { Page } from "@/types";

const PAGES: readonly Page[] = [
  "home",
  "webnovels",
  "manga",
  "books",
  "library",
  "downloads",
  "settings",
  "guardrails",
  "about",
];

function readPage(): Page {
  if (typeof window === "undefined") return "home";
  const value = window.location.hash.replace(/^#\/?/, "");
  return PAGES.includes(value as Page) ? (value as Page) : "home";
}

/**
 * Keeps the active page in the URL hash so deep links, refresh and the
 * browser back button all behave the way a desktop app should.
 */
export function usePageRoute(): [Page, (page: Page) => void] {
  const [page, setPage] = useState<Page>(readPage);

  useEffect(() => {
    const sync = () => setPage(readPage());
    window.addEventListener("hashchange", sync);
    return () => window.removeEventListener("hashchange", sync);
  }, []);

  const navigate = useCallback((next: Page) => {
    setPage(next);
    const hash = `#/${next}`;
    if (window.location.hash !== hash) window.history.pushState(null, "", hash);
  }, []);

  return [page, navigate];
}
