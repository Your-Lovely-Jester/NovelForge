import { ArrowRight, BookOpen, Globe2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { TitlePlate } from "@/components/ui/TitlePlate";

export interface ShelfItem {
  id: string;
  title: string;
  tags: string[];
  description: string;
  chapters?: string;
  source?: string;
  cover?: string;
  coverSeed?: string;
  actionLabel?: string;
  onSelect: () => void;
}

export function ShelfRow({ item, index }: { item: ShelfItem; index: number }) {
  return (
    <article
      className={cn(
        "flex items-start gap-4 border-b border-[#2b282b] py-[7px]",
        index === 0 && "border-t border-[#2b282b] pt-[12px]",
      )}
    >
      {item.cover ? (
        <img
          src={item.cover}
          alt=""
          className="h-[110px] w-[115px] shrink-0 rounded-[5px] object-cover"
          loading="lazy"
        />
      ) : (
        <TitlePlate
          title={item.title}
          source={item.coverSeed ?? item.title}
          className="h-[110px] w-[115px] rounded-[5px]"
        />
      )}

      <div className="min-w-0 flex-1 pt-[6px]">
        <h3 className="truncate font-display text-[23px] leading-[1.05] text-[#eee9e8]">
          {item.title}
        </h3>
        {item.tags.length > 0 && (
          <div className="mt-[9px] flex flex-wrap gap-x-4 gap-y-1 text-[15px] leading-none text-[#969295]">
            {item.tags.map((tag) => (
              <span key={tag}>{tag}</span>
            ))}
          </div>
        )}
        <p className="mt-[9px] line-clamp-2 max-w-[700px] text-[16px] leading-[1.42] text-[#969295]">
          {item.description}
        </p>
      </div>

      <div className="flex h-[110px] w-[200px] shrink-0 flex-col items-stretch justify-between pt-[5px]">
        <div className="flex items-center justify-between gap-3 text-[15px] text-[#969295]">
          {item.chapters ? (
            <span className="flex items-center gap-1.5">
              <BookOpen className="h-[13px] w-[13px]" strokeWidth={1.6} />
              {item.chapters}
            </span>
          ) : (
            <span />
          )}
          {item.source ? (
            <span className="flex items-center gap-1.5 text-[#d8d6d7]">
              <Globe2 className="h-[13px] w-[13px]" strokeWidth={1.6} />
              {item.source}
            </span>
          ) : (
            <span />
          )}
        </div>

        <button
          type="button"
          onClick={item.onSelect}
          className="ml-auto flex h-[35px] w-[90px] items-center justify-center gap-1.5 rounded-[4px] border border-[#3a373a] bg-[#0d0d0f] text-[15px] text-[#d8d6d7] transition-colors hover:border-[#8d3b40] hover:bg-[#1b1214] hover:text-[#eee9e8]"
        >
          {item.actionLabel ?? "Select"}
          <ArrowRight className="h-[13px] w-[13px]" strokeWidth={1.6} />
        </button>
      </div>
    </article>
  );
}
