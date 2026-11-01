import { useRef, useState } from 'react';
import { Download } from 'lucide-react';
import { useBriefingAudio } from './useBriefingAudio';

const SPEEDS = [1, 1.25, 1.5, 2] as const;

/** Plays today's spoken morning briefing; renders nothing when there is none. */
export const BriefingCard = () => {
  const { data } = useBriefingAudio();
  const audioRef = useRef<HTMLAudioElement>(null);
  const [speed, setSpeed] = useState<number>(1);

  if (!data) return null;

  const choose = (next: number) => {
    setSpeed(next);
    if (audioRef.current) audioRef.current.playbackRate = next;
  };

  return (
    <section
      data-testid="briefing-card"
      aria-label="Morning briefing audio"
      className="bg-[var(--color-surface-card)] border border-[var(--color-border-subtle)] rounded-lg p-4 space-y-3"
    >
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-medium text-[var(--color-text-primary)]">Morning briefing</h3>
        <a
          href={data.url}
          download="morning-briefing.ogg"
          className="flex items-center gap-1.5 text-xs text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-colors duration-150 focus-visible:outline focus-visible:outline-2"
        >
          <Download className="w-3.5 h-3.5" />
          Download
        </a>
      </div>
      <audio
        ref={audioRef}
        data-testid="briefing-audio"
        src={data.url}
        controls
        preload="none"
        onLoadedMetadata={() => {
          if (audioRef.current) audioRef.current.playbackRate = speed;
        }}
        className="w-full"
      />
      <div role="group" aria-label="Playback speed" className="flex gap-1.5">
        {SPEEDS.map((s) => (
          <button
            key={s}
            type="button"
            aria-pressed={speed === s}
            onClick={() => choose(s)}
            className={`px-2.5 py-1 rounded-md border text-xs font-mono transition-colors duration-150 cursor-pointer ${
              speed === s
                ? 'border-[var(--color-border-default)] bg-[var(--color-surface-hover)] text-[var(--color-text-primary)]'
                : 'border-[var(--color-border-subtle)] text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)]'
            }`}
          >
            {s}x
          </button>
        ))}
      </div>
    </section>
  );
};
