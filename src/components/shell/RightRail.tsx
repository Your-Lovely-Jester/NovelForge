import type { DownloadTask, LibraryNovel } from "@/types";
import { ProgressCard } from "@/components/rail/ProgressCard";
import { QueueCard } from "@/components/rail/QueueCard";
import { LibraryCard } from "@/components/rail/LibraryCard";

export function RightRail({
  percent,
  activeTasks,
  sessionBytes,
  currentTask,
  queue,
  novels,
  libraryBytes,
  onSelectTask,
  onInspect,
  onPause,
  onResume,
  onCancel,
  onOpenLibrary,
  onOpenFolder,
}: {
  percent: number;
  activeTasks: DownloadTask[];
  sessionBytes: number;
  currentTask: DownloadTask | null;
  queue: DownloadTask[];
  novels: LibraryNovel[];
  libraryBytes: number;
  onSelectTask: (id: string) => void;
  onInspect: () => void;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onCancel: (id: string) => void;
  onOpenLibrary: () => void;
  onOpenFolder: () => void;
}) {
  return (
    <aside className="relative z-10 hidden w-[340px] shrink-0 flex-col gap-[17px] overflow-y-auto py-3 pl-4 pr-4 2xl:w-[380px] 2xl:pl-5 2xl:pr-5 xl:flex">
      <ProgressCard
        percent={percent}
        activeTasks={activeTasks}
        sessionBytes={sessionBytes}
        currentTask={currentTask}
        onInspect={onInspect}
        onPause={onPause}
        onResume={onResume}
      />
      <QueueCard queue={queue} onSelect={onSelectTask} onCancel={onCancel} />
      <LibraryCard
        novels={novels}
        totalBytes={libraryBytes}
        onOpen={onOpenLibrary}
        onOpenFolder={onOpenFolder}
      />
    </aside>
  );
}
