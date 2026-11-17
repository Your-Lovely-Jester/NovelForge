import heroArt from "@/assets/reference/hero.jpg";

/**
 * The reference hero artwork ships with its own title treatment, so the
 * banner is a single image rather than re-set type. The heading stays in the
 * accessibility tree for screen readers.
 */
export function HeroBanner() {
  return (
    <section className="relative w-full overflow-hidden bg-[#09090b]">
      <h1 className="sr-only">Scrape. Read. Enjoy. — your ultimate webnovel scraper.</h1>
      <img
        src={heroArt}
        alt=""
        className="block aspect-[1057/239] w-full object-cover"
        draggable={false}
      />
    </section>
  );
}
