// Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
// SPDX-License-Identifier: GPL-3.0-or-later

import { useEffect, useRef, type ReactNode } from "react";

import { formatAge } from "../lib/format";
import { PRIVACY_NOTES } from "../lib/messages";
import { ProblemReport } from "./ProblemReport";
import type { AppInfo, PrivacyReport } from "../types";

interface HelpProps {
  info: AppInfo | null;
  privacy: PrivacyReport | null;
  /** Errors seen this session, for the problem report. */
  errors: string[];
  /** True when the guide was opened from "Report a problem" in the menu. */
  openReport: boolean;
  /** True when the guide was opened from "Version and updates" in the menu. */
  openVersion: boolean;
  onError: (message: string) => void;
  onClose: () => void;
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="space-y-2">
      <h3 className="text-sm font-semibold uppercase tracking-wide text-zinc-500 dark:text-zinc-400">
        {title}
      </h3>
      <div className="space-y-2 text-sm text-zinc-700 dark:text-zinc-300">
        {children}
      </div>
    </section>
  );
}

/**
 * The application guide.
 *
 * Addresses are rendered as selectable text and not as links: the app has no
 * plugin for opening URLs, and a link that does nothing would be worse than
 * an address you can copy.
 */
export function Help({
  info,
  privacy,
  errors,
  openReport,
  openVersion,
  onError,
  onClose,
}: HelpProps) {
  // Opened from a menu item, the guide should land on the section that was
  // asked for rather than making the reader scroll past five others to find it.
  const reportRef = useRef<HTMLDivElement>(null);
  const versionRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (openReport) {
      reportRef.current?.scrollIntoView({ behavior: "smooth", block: "start" });
    } else if (openVersion) {
      versionRef.current?.scrollIntoView({ behavior: "smooth", block: "start" });
    }
  }, [openReport, openVersion]);

  return (
    <div className="space-y-8">
      <header className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-zinc-100">
            Guide
          </h2>
          <p className="text-sm text-zinc-500 dark:text-zinc-400">
            How to get your data out of Google and put it back in order.
          </p>
        </div>
        <button
          type="button"
          onClick={onClose}
          className="shrink-0 rounded-lg border border-zinc-300 px-3 py-1.5 text-sm font-medium text-zinc-700 transition-colors hover:bg-zinc-100 dark:border-zinc-700 dark:text-zinc-200 dark:hover:bg-zinc-800"
        >
          Close
        </button>
      </header>

      <Section title="1. Getting the export">
        <p>
          Go to <span className="selectable font-mono">takeout.google.com</span>
          , pick the services you care about and start the export. Google takes
          anywhere from a few minutes to several days, and sends you an email
          when it is ready.
        </p>
        <p className="text-zinc-500 dark:text-zinc-400">
          Worth doing: choose the <span className="font-mono">.zip</span> format
          and the largest size available. Fewer archives means fewer chances for
          an interrupted download.
        </p>
      </Section>

      <Section title="2. Loading the export">
        <p>
          Drag the extracted <span className="font-mono">Takeout</span> folder
          into the window, or any one of the{" "}
          <span className="font-mono">takeout-....zip</span> archives: the
          application recognises the others in the same series and merges them
          into a single tree, telling you if one is missing.
        </p>
        <p className="text-zinc-500 dark:text-zinc-400">
          For it to find them, the archives have to sit in the same folder.
          Google splits the export into self contained files that repeat the
          same structure, and the photos of one year can be spread across
          several archives: extracting them one by one into separate folders
          leaves the job half done.
        </p>
        <p>This is the structure it expects:</p>
        <pre className="selectable overflow-x-auto rounded-lg bg-zinc-100 p-3 font-mono text-xs text-zinc-700 dark:bg-zinc-800 dark:text-zinc-300">
          {`Takeout/
├── Google Photos/
│   └── Photos from 2026/
│       ├── IMG_0001.JPG
│       └── IMG_0001.JPG.supplemental-metadata.json
├── Contacts/
├── Calendar/
└── Drive/`}
        </pre>
      </Section>

      <Section title="3. Repairing the photos">
        <p>
          Google exports the capture date and the coordinates into a{" "}
          <span className="font-mono">.json</span> file beside the photograph,
          not inside it. Copy the images anywhere else and that file stays
          behind, and the date is gone: it is why a Takeout poured into a
          gallery shows every photograph dated the day you downloaded it.
        </p>
        <p>
          The repair writes into the EXIF tags everything the{" "}
          <span className="font-mono">.json</span> file holds that has a home in
          metadata: capture date with its time zone, coordinates, description,
          recognised faces and the favourite star. The image is not
          recompressed. It works on JPEG, HEIC, TIFF and WebP. For PNG, GIF and
          video, EXIF is not where metadata lives: there the file date is
          aligned instead and the JSON sidecar is copied alongside, so nothing
          is lost.
        </p>
        <p className="text-zinc-500 dark:text-zinc-400">
          What stays out is only what has nowhere to go in metadata: the view
          count and the address of the photo on Google Photos. They are not
          properties of the photograph, but as long as that JSON exists they are
          there, and the application tells you rather than letting you find out.
        </p>
        <p className="rounded-lg border border-amber-300 bg-amber-50 p-3 text-amber-900 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-200">
          The default mode writes repaired copies into a separate folder and
          does not touch your originals. Rewriting the originals is available,
          but it has to be chosen by hand and confirmed.
        </p>
        <p>
          After the originals have been rewritten the{" "}
          <span className="font-mono">.json</span> files remain in the folder,
          and the application offers to set them aside. It moves only the ones
          that are no longer the sole copy of anything, checking inside each
          photograph that the data really is there: the ones belonging to PNG,
          GIF and video stay, so do the ones for photographs that were not
          repaired and the ones holding data with no tag to live in. It is not a
          deletion, and one click undoes it.
        </p>
      </Section>

      <Section title="4. Cleaning up duplicates">
        <p>
          Exports often contain the same file several times, because a photo in
          three albums is exported three times. The comparison is made on
          content and not on the name: two files with the same name and size but
          different content both survive.
        </p>
        <p>
          <strong className="font-medium text-zinc-900 dark:text-zinc-100">
            No function here deletes a file.
          </strong>{" "}
          You can build a clean tree elsewhere, or move the surplus copies into
          a quarantine folder: in that case a ledger is written and the undo
          button puts every file back where it was.
        </p>
      </Section>

      <Section title="5. When there is not enough room">
        <p>
          A repaired copy is a second library: if the export weighs two hundred
          gigabytes, another two hundred have to be free. The application does
          the arithmetic before starting and stops if it does not fit, rather
          than filling the disk halfway through and leaving a folder that looks
          complete.
        </p>
        <p>
          When the room is not there, two routes are open, and both are offered
          on screen:
        </p>
        <ul className="list-inside list-disc space-y-1 text-zinc-600 dark:text-zinc-400">
          <li>
            <strong className="font-medium text-zinc-900 dark:text-zinc-100">
              Rewrite the originals in place
            </strong>
            : a few tens of megabytes suffice whatever the size of the library,
            because the files are modified one at a time. In exchange no
            untouched copy is left, which is why it has to be confirmed by hand.
          </li>
          <li>
            <strong className="font-medium text-zinc-900 dark:text-zinc-100">
              Work through it in batches
            </strong>
            : the application lists the subfolders, says how much each one
            weighs and which ones fit in the room left, with a button to repair
            one at a time. Free some space, move to the next.
          </li>
        </ul>
        <p className="text-zinc-500 dark:text-zinc-400">
          In that list years and albums are kept apart, because they do not
          weigh the same: an album is mostly copies of photographs that already
          sit in a year folder. Where the application finds files that exist
          only there, it says so: that folder cannot be postponed without losing
          sight of them.
        </p>
      </Section>

      <Section title="6. Taking contacts and calendars with you">
        <p>
          The Contacts and Calendar sections produce a single deduplicated file
          each, in standard vCard 3.0 and iCalendar 2.0, without Google's
          proprietary extensions. They import into Proton, Tuta and Nextcloud
          with nothing in between.
        </p>
      </Section>

      <div ref={reportRef}>
        <Section title="Report a problem">
          <ProblemReport info={info} errors={errors} onError={onError} />
        </Section>
      </div>

      <Section title="Privacy">
        <p>
          The application opens no network connections. That is not a promise
          written here: it is a check that fails the build if anyone introduces
          a library capable of talking to the outside.
        </p>
        {privacy ? (
          <ul className="space-y-1 text-zinc-600 dark:text-zinc-400">
            {privacy.notes.map((note) => (
              <li key={note} className="flex gap-2">
                <span className="text-emerald-600 dark:text-emerald-400">✓</span>
                <span>{PRIVACY_NOTES[note]}</span>
              </li>
            ))}
          </ul>
        ) : null}
        <p className="text-zinc-500 dark:text-zinc-400">
          The web addresses you find in the application, including the ones in
          Drive placeholders, are shown as text and are not clickable: opening
          them would mean a connection, and that is your decision to make,
          outside of here.
        </p>
      </Section>

      <Section title="Time zones">
        <p>
          EXIF tags record the time the clock showed on the spot, without saying
          which zone that was. Google, on the other hand, exports an instant in
          universal time. Writing that instant as it stands would shift every
          photograph: a picture taken in Milan at two in the afternoon would
          appear at one.
        </p>
        <p>
          When the photograph has coordinates, the application works out the
          zone of the place and writes the correct local time together with its
          offset, taking into account the daylight saving in force that day.
          Without coordinates it writes universal time and says so: different
          from the clock, but not ambiguous.
        </p>
        <p className="text-zinc-500 dark:text-zinc-400">
          The time zone boundaries come from OpenStreetMap, distributed under
          the Open Database License (ODbL). The data ships inside the
          application: the lookup happens on your computer and involves no
          connection.
        </p>
      </Section>

      <Section title="Known limits">
        <ul className="list-inside list-disc space-y-1 text-zinc-600 dark:text-zinc-400">
          <li>PNG and GIF: no EXIF writing, only the file date and the sidecar.</li>
          <li>Video: the metadata lives in the container, not in EXIF.</li>
          <li>Mail and YouTube: recognised, but with no dedicated analyser.</li>
          <li>Archives have to be extracted before their sections can be read.</li>
        </ul>
      </Section>

      {info ? (
        <div ref={versionRef}>
          <Section title="Version and updates">
            <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-zinc-600 dark:text-zinc-400">
              <dt>Version</dt>
              <dd className="selectable font-mono">{info.version}</dd>
              <dt>Built on</dt>
              <dd className="selectable font-mono">{info.buildDate}</dd>
              <dt>New versions</dt>
              <dd className="selectable font-mono break-all">
                {info.releasesUrl}
              </dd>
            </dl>
            {info.ageDays >= info.staleAfterDays ? (
              <p className="rounded-lg border border-amber-300 bg-amber-50 px-4 py-3 text-amber-900 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-200">
                This copy was built {formatAge(info.ageDays)} ago. There may be
                a newer one by now: the address above is where they appear.
              </p>
            ) : null}
            <p>
              The application never checks for itself whether an update exists.
              Asking a server would tell it your address and the hours at which
              you open the program, which is the sort of trail this application
              exists to avoid: a check for updates is a beacon like any other.
              The age above costs nothing and reveals nothing, being the
              difference between the compile date and this machine's clock.
            </p>
            <p className="text-zinc-500 dark:text-zinc-400">
              If you installed Nostos from a store or through a package manager,
              there is nothing to do by hand: that is what keeps it current, and
              it is the only part of the system that talks to the network.
            </p>
          </Section>
        </div>
      ) : null}

      {info ? (
        <Section title="About">
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-zinc-600 dark:text-zinc-400">
            <dt>Author</dt>
            <dd className="selectable">{info.author}</dd>
            <dt>Website</dt>
            <dd className="selectable font-mono">{info.homepage}</dd>
            <dt>Source</dt>
            <dd className="selectable font-mono break-all">{info.repository}</dd>
            <dt>Licence</dt>
            <dd className="selectable font-mono">{info.license}</dd>
          </dl>
          <p className="text-zinc-500 dark:text-zinc-400">
            Free software: you may use it, study it, modify it and redistribute
            it. The licence requires every derived version to stay just as free,
            so nobody can take this code, add tracking to it and ship it as a
            closed program.
          </p>
          <p className="text-zinc-500 dark:text-zinc-400">
            This application is not affiliated with, endorsed by or sponsored by
            Google LLC. Google, Google Photos, Google Drive and Google Takeout
            are trademarks of Google LLC, named here only to identify the export
            this software reads.
          </p>
        </Section>
      ) : null}
    </div>
  );
}
