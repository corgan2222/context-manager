import Link from 'next/link';
import { ImageZoom } from 'fumadocs-ui/components/image-zoom';
import { ContextMenuArt } from '@/components/context-menu-art';
// The same file the documentation shows, imported rather than linked: a path
// written by hand keeps the path it was written with, and the site is served
// under /context-manager.
import overview from '../../../public/images/01-overview_en.web.png';

const REPO = 'https://github.com/corgan2222/context-manager';

const reasons = [
  {
    title: 'A backup is a type, not a promise',
    body: 'Every destructive call takes a token that only a successful backup produces. "Never delete without a backup" is checked by the compiler, not by discipline.',
  },
  {
    title: 'The whole file-type chain',
    body: 'For one extension the scan walks all seven levels Windows walks — user choice, ProgID, perceived type, SystemFileAssociations. The two levels most editors overlook are where image tools register.',
  },
  {
    title: 'Two hundred tools from one address',
    body: 'Paste the docs URL of any service that describes itself through OpenAPI. Every endpoint that takes a file becomes a menu entry, grouped the way the service groups itself.',
  },
  {
    title: 'The new Windows 11 menu, too',
    body: 'Packaged entries are read from their manifests and hidden through the per-user blocked list. Your own entries reach the upper menu through a handler the executable carries inside itself.',
  },
  {
    title: 'Signed updates',
    body: 'The program checks a signature made with a key that does not live in the repository, then the published checksum. A release without both is never offered.',
  },
  {
    title: 'One file, no installer',
    body: 'A single Rust executable. No runtime library, no background service, no registry key of its own until you create one.',
  },
];

// The short list, for somebody who is scanning rather than reading. The
// footnote markers are not decoration: both claims have a real edge to them,
// and a bullet that hides it is a bullet somebody feels lied to by later.
const points = [
  'Free and open source',
  'MIT licence',
  ['Windows 11 ready', '1'],
  ['Works without administrator rights', '2'],
  'Four switches instead of a row of verbs',
  'Automatic backups before every change',
  'Updates itself, signature checked first',
  'A favourites list you place from',
  'Dark and light mode',
  'German and English, switched at runtime',
  'Add entries by drag and drop',
  'Search across names, paths and commands',
  'Finds entries whose program is gone',
  'Built-in services from any OpenAPI address',
  'Built-in help, with working command lines',
  'Built in Rust: one file, no runtime library',
];

/* The band under the hero: what the program runs on, what it is built with,
 * and what it reaches out to. Marks rather than counts, because a count taken
 * on one machine says nothing about the reader's. */
const marks = [
  { mark: <WindowsElevenMark />, label: 'Windows 11' },
  { mark: <WindowsTenMark />, label: 'Windows 10' },
  { mark: <RustMark />, label: 'Built in Rust' },
  { mark: <ServicesMark />, label: 'Services' },
];

function ServicesMark() {
  /* The same cloud the services page carries in the sidebar, so the two read
   * as one thing. */
  return (
    <svg
      viewBox="0 0 24 24"
      className="size-8"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M17.5 19H7a5 5 0 1 1 1.1-9.88A6 6 0 0 1 19.6 10.4 3.8 3.8 0 0 1 17.5 19Z" />
    </svg>
  );
}

function WindowsElevenMark() {
  return (
    <svg viewBox="0 0 24 24" className="size-8" fill="currentColor" aria-hidden="true">
      <rect x="1.5" y="1.5" width="9.4" height="9.4" rx="0.6" />
      <rect x="13.1" y="1.5" width="9.4" height="9.4" rx="0.6" />
      <rect x="1.5" y="13.1" width="9.4" height="9.4" rx="0.6" />
      <rect x="13.1" y="13.1" width="9.4" height="9.4" rx="0.6" />
    </svg>
  );
}

function WindowsTenMark() {
  /* The tilted grid: the outer edges rise to the right, which is the whole
   * difference to the Windows 11 mark above. */
  return (
    <svg viewBox="0 0 24 24" className="size-8" fill="currentColor" aria-hidden="true">
      <path d="M1 4.1 11 2.7 V11.2 H1 Z" />
      <path d="M12.4 2.5 23 1 V11.2 H12.4 Z" />
      <path d="M1 12.8 H11 V21.3 L1 19.9 Z" />
      <path d="M12.4 12.8 H23 V23 L12.4 21.5 Z" />
    </svg>
  );
}

/* Twelve teeth, outer radius 11.3, inner 8.9, each tooth spanning 17° of its
 * 30° step, drawn around (12,12). Written out rather than computed at render
 * time, and rebuilt from those five numbers if it ever needs to change. A
 * dashed circle was the shorter way and had to go: at 32px the dashes closed
 * up and the whole mark read as a ®. */
const gearTeeth =
  'M23.2 10.3L23.2 13.7L20.8 13.3L20.3 15.3L22.5 16.1L20.8 19.0L19.0 17.5L17.5 19.0L19.0 20.8L16.1 22.5L15.3 20.3L13.3 20.8L13.7 23.2L10.3 23.2L10.7 20.8L8.7 20.3L7.9 22.5L5.0 20.8L6.5 19.0L5.0 17.5L3.2 19.0L1.5 16.1L3.7 15.3L3.2 13.3L0.8 13.7L0.8 10.3L3.2 10.7L3.7 8.7L1.5 7.9L3.2 5.0L5.0 6.5L6.5 5.0L5.0 3.2L7.9 1.5L8.7 3.7L10.7 3.2L10.3 0.8L13.7 0.8L13.3 3.2L15.3 3.7L16.1 1.5L19.0 3.2L17.5 5.0L19.0 6.5L20.8 5.0L22.5 7.9L20.3 8.7L20.8 10.7Z';

function RustMark() {
  return (
    <svg viewBox="0 0 24 24" className="size-8" aria-hidden="true">
      {/* The ring is the teeth with a round hole punched out of it, which is
        * what `evenodd` is for. The R then sits in the hole. */}
      <path
        fill="currentColor"
        fillRule="evenodd"
        d={`${gearTeeth} M12 4.6a7.4 7.4 0 1 0 0 14.8 7.4 7.4 0 1 0 0-14.8Z`}
      />
      <g
        fill="none"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d="M9.4 16.5 V7.7 h3.5 a2.3 2.3 0 0 1 0 4.6 H9.4" />
        <path d="M12.5 12.3 14.9 16.5" />
      </g>
    </svg>
  );
}

export default function HomePage() {
  return (
    <main className="flex flex-1 flex-col">
      <section className="hero-wash overflow-hidden">
        <div className="mx-auto flex w-full max-w-7xl flex-col items-center justify-center gap-12 px-4 pt-24 pb-16 text-center xl:flex-row xl:gap-16 xl:text-left">
          <ContextMenuArt className="hidden xl:block" />
          <div className="flex max-w-2xl flex-col items-center xl:items-start">
            <span className="mb-5 rounded-full border border-fd-border bg-fd-card px-4 py-1.5 text-sm text-fd-muted-foreground">
              Windows 10 and 11 · 64-bit · v1.5.0
            </span>
            <h1 className="text-balance text-4xl font-bold tracking-tight sm:text-6xl">
              Take your right-click menu back
            </h1>
            <p className="mt-6 text-balance text-lg text-fd-muted-foreground">
              See every entry, where it lives in the registry and which program
              put it there. Hide it, sort it, delete it, or build your own — and
              every change is backed up before it happens.
            </p>
            <div className="mt-9 flex flex-wrap items-center justify-center gap-3 xl:justify-start">
              <a
                href={`${REPO}/releases/latest`}
                className="rounded-xl bg-fd-primary px-6 py-3 font-medium text-white transition-opacity hover:opacity-90"
              >
                Download ctxmenu.exe
              </a>
              <Link
                href="/docs"
                className="rounded-xl border border-fd-border bg-fd-card px-6 py-3 font-medium transition-colors hover:bg-fd-accent"
              >
                Read the documentation
              </Link>
            </div>
            <p className="mt-4 text-sm text-fd-muted-foreground">
              No installer. No runtime library. No background service.
            </p>
          </div>
        </div>
      </section>

      <section className="border-y border-fd-border bg-fd-card/40">
        <div className="mx-auto grid max-w-5xl grid-cols-2 gap-8 px-6 py-10 md:grid-cols-4">
          {marks.map((m) => (
            <div key={m.label} className="flex flex-col items-center text-center">
              <div className="flex h-9 items-center text-3xl font-bold tracking-tight">
                {m.mark}
              </div>
              <div className="mt-1 text-sm text-fd-muted-foreground">
                {m.label}
              </div>
            </div>
          ))}
        </div>
      </section>

      <section className="mx-auto w-full max-w-6xl px-6 pt-16">
        <div className="overflow-hidden rounded-2xl border border-fd-border bg-fd-card shadow-2xl">
          <ImageZoom
            src={overview}
            alt="The Categories tab: the tree of categories on the left, the entries of the selected one in the table, and the detail pane below it."
            sizes="(max-width: 1152px) 100vw, 1152px"
            priority
          />
        </div>
        <p className="mt-3 text-center text-sm text-fd-muted-foreground">
          The Categories tab. Click to see it full size.
        </p>
      </section>

      <section className="mx-auto w-full max-w-7xl px-6 pt-16">
        <ul className="grid gap-x-10 gap-y-3 sm:grid-cols-2 lg:grid-cols-3">
          {points.map((p) => {
            const [label, note] = Array.isArray(p) ? p : [p, null];
            return (
              <li key={label} className="flex items-start gap-2.5 text-sm">
                <svg
                  className="mt-0.5 size-4 shrink-0 text-fd-primary"
                  viewBox="0 0 20 20"
                  fill="currentColor"
                  aria-hidden="true"
                >
                  <path
                    fillRule="evenodd"
                    d="M16.7 5.3a1 1 0 0 1 0 1.4l-7.5 7.5a1 1 0 0 1-1.4 0L3.3 9.7a1 1 0 1 1 1.4-1.4l3.8 3.8 6.8-6.8a1 1 0 0 1 1.4 0Z"
                    clipRule="evenodd"
                  />
                </svg>
                <span className="lg:whitespace-nowrap">
                  {label}
                  {note && (
                    <sup className="ms-0.5 text-fd-muted-foreground">{note}</sup>
                  )}
                </span>
              </li>
            );
          })}
        </ul>
        <div className="mt-8 space-y-1.5 border-t border-fd-border pt-5 text-xs text-fd-muted-foreground">
          <p>
            <sup>1</sup> The entries of the Windows 11 menu are listed, hidden
            and created. Their order up there belongs to Explorer, and the
            commands it builds in itself carry no registration to reach.
          </p>
          <p>
            <sup>2</sup> Your own entries always go to <code>HKCU</code>.
            Changing an entry that lives under <code>HKLM</code> needs
            elevation, and it is asked for that one step only.
          </p>
        </div>
      </section>

      <section className="mx-auto w-full max-w-6xl px-6 py-20">
        <h2 className="text-center text-3xl font-bold tracking-tight">
          Why this one
        </h2>
        <p className="mx-auto mt-3 max-w-2xl text-center text-fd-muted-foreground">
          Six things that are mechanism rather than marketing.
        </p>
        <div className="mt-12 grid gap-5 md:grid-cols-2 lg:grid-cols-3">
          {reasons.map((r) => (
            <article
              key={r.title}
              className="rounded-2xl border border-fd-border bg-fd-card p-6 transition-colors hover:border-fd-primary/40"
            >
              <h3 className="font-semibold">{r.title}</h3>
              <p className="mt-2 text-sm leading-relaxed text-fd-muted-foreground">
                {r.body}
              </p>
            </article>
          ))}
        </div>
      </section>

      <section className="border-t border-fd-border">
        <div className="mx-auto flex max-w-5xl flex-col items-center gap-4 px-6 py-16 text-center">
          <h2 className="text-2xl font-bold tracking-tight">
            Every change is backed up first
          </h2>
          <p className="max-w-2xl text-fd-muted-foreground">
            Not as a matter of policy. The delete function cannot be called
            without proof that a backup exists, and the proof is a value only a
            successful export hands out.
          </p>
          <Link
            href="/docs/backups"
            className="mt-2 rounded-xl border border-fd-border bg-fd-card px-5 py-2.5 text-sm font-medium transition-colors hover:bg-fd-accent"
          >
            How the backups work
          </Link>
        </div>
      </section>
    </main>
  );
}
