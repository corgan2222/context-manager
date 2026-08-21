import type { ReactNode } from 'react';

/* The hero artwork: the Windows 11 menu the program builds, drawn rather than
 * photographed.
 *
 * A screenshot would have to be retaken for every release and would carry
 * whatever else sat on that machine. This is markup, so it stays right, scales
 * without blurring and reads in both themes. Decorative throughout, hence one
 * `aria-hidden` on the outermost element and no text a reader would announce.
 *
 * The animation runs on a fourteen-second loop: the antivirus entry folds
 * away, then the two own entries unfold in its place. Motion lives in
 * `global.css` so `prefers-reduced-motion` can switch it off in one rule. */

type Line =
  | 'separator'
  | {
      icon: keyof typeof icons;
      label: string;
      /** Right-aligned accelerator, as Explorer prints it. */
      hint?: string;
      /** A submenu arrow rather than an accelerator. */
      submenu?: boolean;
      /** The pill that says who owns the entry, or that it is switched off. */
      tag?: 'mine' | 'off';
      /** Name of the keyframes that fold this line in or out. */
      animation?: string;
      highlight?: boolean;
    };

/* Every drawing here is decoration. `aria-hidden` is written out on each svg
 * rather than kept in this object: a reader walks the tree, and the lint rule
 * that enforces it reads the tag, not a spread. */
const svgProps = {
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.3,
  strokeLinecap: 'round',
  strokeLinejoin: 'round',
} as const;

const icons = {
  cut: (
    <>
      <circle cx="4.2" cy="4.4" r="1.7" />
      <circle cx="4.2" cy="11.6" r="1.7" />
      <path d="M5.7 5.5 L13.3 12.4 M5.7 10.5 L13.3 3.6" />
    </>
  ),
  copy: (
    <>
      <rect x="5.6" y="5.6" width="7.6" height="7.6" rx="1.4" />
      <path d="M3.2 10.6 V4.7 a1.5 1.5 0 0 1 1.5 -1.5 h5.9" />
    </>
  ),
  rename: (
    <>
      <path d="M8 3 v10 M6.4 3 h3.2 M6.4 13 h3.2" />
      <path d="M5 5.6 H4.2 A1.4 1.4 0 0 0 2.8 7 v2 a1.4 1.4 0 0 0 1.4 1.4 H5 M11 5.6 h.8 A1.4 1.4 0 0 1 13.2 7 v2 a1.4 1.4 0 0 1 -1.4 1.4 H11" />
    </>
  ),
  share: (
    <>
      <path d="M8 9.8 V3 M5.6 5.2 8 2.8 l2.4 2.4" />
      <path d="M4.8 8.2 H4.2 A1.6 1.6 0 0 0 2.6 9.8 v1.6 a1.6 1.6 0 0 0 1.6 1.6 h7.6 a1.6 1.6 0 0 0 1.6 -1.6 V9.8 a1.6 1.6 0 0 0 -1.6 -1.6 h-.6" />
    </>
  ),
  trash: (
    <path d="M3.2 4.6 h9.6 M6.4 4.6 V3.4 a.8 .8 0 0 1 .8 -.8 h1.6 a.8 .8 0 0 1 .8 .8 v1.2 M4.6 4.6 l.5 7.6 a1.3 1.3 0 0 0 1.3 1.2 h3.2 a1.3 1.3 0 0 0 1.3 -1.2 l.5 -7.6" />
  ),
  file: (
    <>
      <path d="M9.2 2.6 H5.2 a1.2 1.2 0 0 0 -1.2 1.2 v8.4 a1.2 1.2 0 0 0 1.2 1.2 h5.6 a1.2 1.2 0 0 0 1.2 -1.2 V5.4 Z" />
      <path d="M9.2 2.6 V5.4 h2.8" />
    </>
  ),
  apps: (
    <>
      <rect x="3" y="3" width="4.2" height="4.2" rx="1" />
      <rect x="8.8" y="3" width="4.2" height="4.2" rx="1" />
      <rect x="3" y="8.8" width="4.2" height="4.2" rx="1" />
      <path d="M10.9 9.2 v3.6 M9.1 11 h3.6" />
    </>
  ),
  path: <path d="M5.4 5 L2.8 8 l2.6 3 M10.6 5 L13.2 8 l-2.6 3" />,
  properties: (
    <>
      <path d="M3 5.2 h10 M3 10.8 h10" />
      <circle cx="6.2" cy="5.2" r="1.5" fill="#2b2f3a" />
      <circle cx="9.8" cy="10.8" r="1.5" fill="#2b2f3a" />
    </>
  ),
  notepad: (
    <>
      <rect x="3.5" y="2.5" width="9" height="11" rx="1.2" />
      <path d="M5.5 5.5 h5 M5.5 8 h5 M5.5 10.5 h3" />
    </>
  ),
  pencil: (
    <path d="M12.6 3.4 a1.5 1.5 0 0 0 -2.1 0 L5 8.9 l-.9 3 3 -.9 5.5 -5.5 a1.5 1.5 0 0 0 0 -2.1 Z" />
  ),
  upload: (
    <>
      <circle cx="8" cy="8" r="5.4" />
      <path d="M8 10.6 V5.8 M6 7.6 8 5.6 l2 2" />
    </>
  ),
  edit: (
    <>
      <rect x="2.8" y="2.8" width="10.4" height="10.4" rx="1.6" />
      <path d="M10 5.2 l.8 .8 -4.2 4.2 -1.4 .6 .6 -1.4 Z" />
    </>
  ),
  shield: (
    <path d="M8 2.6 l4.6 1.8 v3.2 c0 2.8 -2 4.6 -4.6 5.8 c-2.6 -1.2 -4.6 -3 -4.6 -5.8 V4.4 Z" />
  ),
  photos: (
    <>
      <rect x="2.8" y="3.5" width="10.4" height="9" rx="1.4" />
      <circle cx="5.8" cy="6.4" r="1" />
      <path d="M4.2 11.2 l2.8 -2.8 2 2 2.3 -2.3 1.7 1.7" />
    </>
  ),
  more: (
    <g fill="currentColor" stroke="none">
      <circle cx="3.6" cy="8" r="1.15" />
      <circle cx="8" cy="8" r="1.15" />
      <circle cx="12.4" cy="8" r="1.15" />
    </g>
  ),
} satisfies Record<string, ReactNode>;

function Icon({ name, size = 16 }: { name: keyof typeof icons; size?: number }) {
  return (
    <svg viewBox="0 0 16 16" width={size} height={size} aria-hidden="true" {...svgProps}>
      {icons[name]}
    </svg>
  );
}

/* The five verbs Windows 11 put in a row of icons at the top of the menu. */
const verbs = [
  { icon: 'cut', label: 'Cut' },
  { icon: 'copy', label: 'Copy' },
  { icon: 'rename', label: 'Rename' },
  { icon: 'share', label: 'Share' },
  { icon: 'trash', label: 'Delete' },
] as const;

const lines: Line[] = [
  { icon: 'file', label: 'Open', hint: 'Enter' },
  { icon: 'apps', label: 'Open with', submenu: true },
  'separator',
  { icon: 'path', label: 'Copy as path', hint: 'Ctrl+Shift+C' },
  { icon: 'properties', label: 'Properties', hint: 'Alt+Enter' },
  'separator',
  { icon: 'notepad', label: 'Edit in Notepad' },
  { icon: 'pencil', label: 'Edit with Paint' },
  {
    icon: 'upload',
    label: 'Upload with ShareX',
    tag: 'mine',
    animation: 'ctxSxIn',
    highlight: true,
  },
  { icon: 'edit', label: 'Edit with Photoshop', animation: 'ctxPsIn' },
  {
    icon: 'shield',
    label: 'Scan with AntiVirus Plus',
    tag: 'off',
    animation: 'ctxAvOut',
  },
  { icon: 'photos', label: 'Photos', submenu: true },
  'separator',
  { icon: 'more', label: 'Show more options', hint: 'Shift+F10' },
];

/* The menu as it was before: everything every installer ever added, stacked
 * into one column. Blurred and tilted, so it reads as the state being left
 * behind rather than as a second thing to look at. */
const before = [
  'Open',
  'Open with',
  'Edit with Paint 3D',
  'separator',
  'Scan with AntiVirus Plus',
  'Share with SkyMeet',
  'Add to archive…',
  'Add to "screenshot.zip"',
  'Compress and email…',
  'Convert to PDF',
  'Convert to PDF and email',
  'Send to OneNote',
  'separator',
  'Restore previous versions',
  'Include in library',
  'Pin to Start',
  'Send to',
  'separator',
  'Copy as path',
  'Create shortcut',
  'Delete',
  'Rename',
  'separator',
  'Properties',
];

const chevron = (
  <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true" {...svgProps} strokeWidth={1.6}>
    <path d="M6.2 4.2 L10 8 l-3.8 3.8" />
  </svg>
);

function Tag({ kind }: { kind: 'mine' | 'off' }) {
  if (kind === 'mine') {
    return (
      <span className="ctx-tag ctx-tag-mine">yours · HKCU</span>
    );
  }
  return (
    <span className="ctx-tag-group">
      <span className="ctx-tag">off</span>
      <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true" {...svgProps} strokeWidth={1.2}>
        <path d="M2.8 8 c1.6 -2.6 3.4 -3.8 5.2 -3.8 s3.6 1.2 5.2 3.8 c-1.6 2.6 -3.4 3.8 -5.2 3.8 s-3.6 -1.2 -5.2 -3.8 Z" />
        <path d="M3.6 12.4 L12.4 3.6" />
      </svg>
    </span>
  );
}

export function ContextMenuArt({ className }: { className?: string }) {
  return (
    <div className={`ctx-art ${className ?? ''}`} aria-hidden="true">
      <div className="ctx-glow" />

      <div className="ctx-before">
        {before.map((entry, i) =>
          entry === 'separator' ? (
            // biome-ignore lint/suspicious/noArrayIndexKey: fixed decorative list
            <div key={i} className="ctx-rule" />
          ) : (
            <div key={entry} className="ctx-before-row">
              <span>{entry}</span>
              {(entry === 'Open with' ||
                entry === 'Include in library' ||
                entry === 'Send to') && <span className="ctx-arrow">›</span>}
            </div>
          ),
        )}
      </div>

      <div className="ctx-tilt">
        <div className="ctx-menu">
          <div className="ctx-verbs">
            {verbs.map((v) => (
              <div key={v.label} className="ctx-verb">
                <Icon name={v.icon} size={17} />
                <span>{v.label}</span>
              </div>
            ))}
          </div>
          <div className="ctx-rule" />

          {lines.map((line, i) =>
            line === 'separator' ? (
              // biome-ignore lint/suspicious/noArrayIndexKey: fixed decorative list
              <div key={i} className="ctx-rule" />
            ) : (
              <div
                key={line.label}
                className={`ctx-row${line.highlight ? ' ctx-row-mine' : ''}`}
                style={
                  line.animation
                    ? { animation: `${line.animation} 14s ease-in-out infinite` }
                    : undefined
                }
              >
                <span className="ctx-icon">
                  <Icon name={line.icon} />
                </span>
                <span className="ctx-label">{line.label}</span>
                {line.tag && <Tag kind={line.tag} />}
                {line.hint && <span className="ctx-hint">{line.hint}</span>}
                {line.submenu && <span className="ctx-hint">{chevron}</span>}
              </div>
            ),
          )}
        </div>
      </div>

      <div className="ctx-cursor">
        <svg viewBox="0 0 20 22" width="20" height="22" aria-hidden="true">
          <path
            d="M4 1 L4 16.5 L8.2 12.9 L10.6 18.6 L13.6 17.3 L11.2 11.7 L16.8 11.2 Z"
            fill="#ffffff"
            stroke="#22262e"
            strokeWidth="1.2"
          />
        </svg>
      </div>

      <div className="ctx-toast">
        <span className="ctx-toast-mark">
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" {...svgProps} strokeWidth={1.6}>
            <path d="M3.6 8.4 6.4 11.2 12.4 4.8" />
          </svg>
        </span>
        <span className="ctx-toast-text">
          <span className="ctx-toast-title">Backup created</span>
          <span className="ctx-toast-path">
            {String.raw`HKCU\…\shell\ShareX`} — restore anytime
          </span>
        </span>
      </div>
    </div>
  );
}
