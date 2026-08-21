import Link from 'next/link';

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
  'Search across names, paths, commands and CLSIDs',
  'Finds entries whose program is gone',
  'Built-in services from any OpenAPI address',
  'Built-in help, with working command lines',
  'Built in Rust: one file, no runtime library',
];

const numbers = [
  { value: '11', label: 'base categories scanned' },
  { value: '938', label: 'entries found on a grown machine' },
  { value: '7', label: 'levels of file-type resolution' },
  { value: '0', label: 'changes without a backup' },
];

export default function HomePage() {
  return (
    <main className="flex flex-1 flex-col">
      <section className="hero-wash flex flex-col items-center px-4 pt-24 pb-16 text-center">
        <span className="mb-5 rounded-full border border-fd-border bg-fd-card px-4 py-1.5 text-sm text-fd-muted-foreground">
          Windows 10 and 11 · 64-bit · v1.5.0
        </span>
        <h1 className="max-w-3xl text-balance text-4xl font-bold tracking-tight sm:text-6xl">
          Take your right-click menu back
        </h1>
        <p className="mt-6 max-w-2xl text-balance text-lg text-fd-muted-foreground">
          See every entry, where it lives in the registry and which program put
          it there. Hide it, sort it, delete it, or build your own — and every
          change is backed up before it happens.
        </p>
        <div className="mt-9 flex flex-wrap items-center justify-center gap-3">
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
      </section>

      <section className="border-y border-fd-border bg-fd-card/40">
        <div className="mx-auto grid max-w-5xl grid-cols-2 gap-8 px-6 py-10 md:grid-cols-4">
          {numbers.map((n) => (
            <div key={n.label} className="text-center">
              <div className="text-3xl font-bold tracking-tight">{n.value}</div>
              <div className="mt-1 text-sm text-fd-muted-foreground">
                {n.label}
              </div>
            </div>
          ))}
        </div>
      </section>

      <section className="mx-auto w-full max-w-5xl px-6 pt-16">
        <ul className="grid gap-x-8 gap-y-3 sm:grid-cols-2 lg:grid-cols-3">
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
                <span>
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
