import React, { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import {
  AlertTriangle,
  Check,
  Copy,
  KeyRound,
  Link2,
  Loader2,
  LogOut,
  Monitor,
  ShieldAlert,
  Smartphone,
  Tablet,
  Trash2,
} from 'lucide-react';
import {
  describeEvent,
  isAlarming,
  securityApi,
  SECURITY_KEYS,
  useDeviceSessions,
  useMatrixIdentities,
  usePasskeys,
  useRecoveryStatus,
  useSecurityEvents,
  useSensitiveAction,
} from '../core/security';
import { addPasskey, describePasskeyError } from '../core/passkey';
import { formatDateTimeInZone } from '../core/settings';
import { copyToClipboard } from '../core/clipboard';
import { useSettings } from '../core/settings';

const Section: React.FC<{ title: string; hint?: string; children: React.ReactNode }> = ({
  title,
  hint,
  children,
}) => (
  <section className="flex flex-col gap-3 border-t border-neutral-200 pt-5 dark:border-neutral-800">
    <div className="flex flex-col gap-1">
      <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">{title}</h3>
      {hint && <p className="text-xs text-neutral-500 dark:text-neutral-400">{hint}</p>}
    </div>
    {children}
  </section>
);

const Button: React.FC<
  React.ButtonHTMLAttributes<HTMLButtonElement> & { busy?: boolean; tone?: 'default' | 'danger' }
> = ({ busy, tone = 'default', children, className = '', ...rest }) => (
  <button
    type="button"
    {...rest}
    disabled={rest.disabled || busy}
    className={`inline-flex items-center justify-center gap-2 rounded-lg px-3 py-2 text-sm font-medium disabled:opacity-60 ${
      tone === 'danger'
        ? 'bg-red-600 text-white hover:bg-red-700'
        : 'bg-neutral-900 text-white hover:bg-neutral-700 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-300'
    } ${className}`}
  >
    {busy && <Loader2 size={14} className="animate-spin" />}
    {children}
  </button>
);

interface ParsedDevice {
  name: string;
  type: 'desktop' | 'mobile' | 'tablet';
}

function parseDevice(userAgent?: string | null): ParsedDevice {
  if (!userAgent) {
    return { name: 'Unknown device', type: 'desktop' };
  }

  // Preserve pre-formatted strings used in tests like "Chrome on macOS" or "Firefox on Linux"
  if (userAgent.includes(' on ')) {
    const isMobile = /iphone|android|mobile/i.test(userAgent);
    const isTablet = /ipad|tablet/i.test(userAgent);
    return {
      name: userAgent,
      type: isTablet ? 'tablet' : isMobile ? 'mobile' : 'desktop',
    };
  }

  const ua = userAgent;

  // OS detection
  let os = '';
  let type: 'desktop' | 'mobile' | 'tablet' = 'desktop';

  if (/windows nt 10/i.test(ua)) {
    os = 'Windows';
  } else if (/windows nt 6\.3/i.test(ua)) {
    os = 'Windows 8.1';
  } else if (/windows nt 6\.1/i.test(ua)) {
    os = 'Windows 7';
  } else if (/windows/i.test(ua)) {
    os = 'Windows';
  } else if (/iphone/i.test(ua)) {
    os = 'iPhone';
    type = 'mobile';
  } else if (/ipad/i.test(ua)) {
    os = 'iPad';
    type = 'tablet';
  } else if (/macintosh|mac os x/i.test(ua)) {
    os = 'macOS';
  } else if (/android/i.test(ua)) {
    if (/mobile/i.test(ua)) {
      os = 'Android';
      type = 'mobile';
    } else {
      os = 'Android Tablet';
      type = 'tablet';
    }
  } else if (/cros/i.test(ua)) {
    os = 'ChromeOS';
  } else if (/linux/i.test(ua)) {
    os = 'Linux';
  }

  // Browser detection
  let browser = '';
  if (/edg(?:e)?\/([0-9.]+)/i.test(ua)) {
    browser = 'Edge';
  } else if (/opr\/|opera\/([0-9.]+)/i.test(ua)) {
    browser = 'Opera';
  } else if (/brave/i.test(ua)) {
    browser = 'Brave';
  } else if (/chrome\/([0-9.]+)/i.test(ua) && !/chromium/i.test(ua)) {
    browser = 'Chrome';
  } else if (/firefox\/([0-9.]+)/i.test(ua)) {
    browser = 'Firefox';
  } else if (/version\/([0-9.]+).*safari/i.test(ua)) {
    browser = 'Safari';
  } else if (/safari/i.test(ua)) {
    browser = 'Safari';
  }

  if (browser && os) {
    return { name: `${browser} on ${os}`, type };
  }
  if (browser) {
    return { name: browser, type };
  }
  if (os) {
    return { name: `${os} Device`, type };
  }

  return { name: 'Unknown device', type };
}

/**
 * Settings → Security.
 *
 * Everything here is gated on a recent passkey. Rather than surfacing the
 * server's 403 as an error, `useSensitiveAction` re-prompts and retries — so
 * from the user's side a sensitive action sometimes asks for their passkey, and
 * that is the whole of it.
 */
export const SecurityPanel: React.FC = () => {
  const settings = useSettings();
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const [freshCodes, setFreshCodes] = useState<string[] | null>(null);
  const [isAddingPasskey, setIsAddingPasskey] = useState(false);
  const [copiedTarget, setCopiedTarget] = useState<string | null>(null);

  const handleCopy = async (text: string, target: string) => {
    const ok = await copyToClipboard(text);
    if (ok) {
      setCopiedTarget(target);
      setTimeout(() => setCopiedTarget(null), 2000);
    }
  };

  const passkeys = usePasskeys();
  const links = useMatrixIdentities();
  const sessions = useDeviceSessions();
  const events = useSecurityEvents();
  const recovery = useRecoveryStatus();

  const fmt = (iso: string | null) =>
    iso ? formatDateTimeInZone(iso, settings.timezone) : '—';

  const handleAddPasskey = async () => {
    setIsAddingPasskey(true);
    setError(null);
    try {
      await addPasskey();
      queryClient.invalidateQueries({ queryKey: SECURITY_KEYS.passkeys });
      queryClient.invalidateQueries({ queryKey: SECURITY_KEYS.events });
    } catch (err) {
      setError(describePasskeyError(err));
    } finally {
      setIsAddingPasskey(false);
    }
  };

  const run = <T,>(m: { mutateAsync: (a: T) => Promise<unknown>; isPending: boolean }, arg: T) => {
    setError(null);
    m.mutateAsync(arg).catch((err) => setError(describePasskeyError(err)));
  };

  const unlink = useSensitiveAction(securityApi.unlinkMatrix, [SECURITY_KEYS.identities, SECURITY_KEYS.events]);
  const deletePasskey = useSensitiveAction(securityApi.deletePasskey, [
    SECURITY_KEYS.passkeys,
    SECURITY_KEYS.events,
  ]);
  const revokeOthers = useSensitiveAction(securityApi.revokeOtherSessions, [
    SECURITY_KEYS.sessions,
    SECURITY_KEYS.events,
  ]);
  const revokeSession = useSensitiveAction(securityApi.revokeSession, [
    SECURITY_KEYS.sessions,
    SECURITY_KEYS.events,
  ]);
  const regenerate = useSensitiveAction(securityApi.regenerateRecoveryCodes, [
    SECURITY_KEYS.recovery,
    SECURITY_KEYS.events,
  ]);
  const cancelRecovery = useSensitiveAction(securityApi.cancelRecovery, [
    SECURITY_KEYS.recovery,
    SECURITY_KEYS.events,
  ]);

  const pendingRecovery = recovery.data?.recovery_valid_after ?? null;

  return (
    <div className="flex flex-col gap-5">
      {error && (
        <p role="alert" className="rounded-lg bg-red-50 p-3 text-sm text-red-700 dark:bg-red-950 dark:text-red-200">
          {error}
        </p>
      )}

      {/* The one thing on this screen that may be an attack in progress. */}
      {pendingRecovery && (
        <div className="flex flex-col gap-3 rounded-lg border border-red-300 bg-red-50 p-4 dark:border-red-800 dark:bg-red-950">
          <div className="flex items-start gap-3">
            <ShieldAlert size={20} className="mt-0.5 shrink-0 text-red-600 dark:text-red-400" />
            <div className="text-sm text-red-800 dark:text-red-200">
              <p className="font-semibold">Account recovery was requested.</p>
              <p>
                It takes effect {fmt(pendingRecovery)}. If this was not you, cancel it now — and
                assume whoever asked has access to one of your linked Matrix accounts.
              </p>
            </div>
          </div>
          <Button
            tone="danger"
            busy={cancelRecovery.isPending}
            onClick={() => run(cancelRecovery, undefined as void)}
            className="self-start"
          >
            Cancel recovery
          </Button>
        </div>
      )}

      <Section
        title="Passkeys"
        hint="How you sign in. Keep at least two, on different devices."
      >
        <ul className="flex flex-col gap-2">
          {passkeys.data?.map((key) => (
            <li
              key={key.id}
              className="flex items-center gap-3 rounded-lg bg-neutral-50 px-3 py-2 dark:bg-neutral-800"
            >
              <KeyRound size={16} className="shrink-0 text-neutral-500" />
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm text-neutral-900 dark:text-neutral-100">{key.label}</p>
                <p className="text-xs text-neutral-500 dark:text-neutral-400">
                  Added {fmt(key.created_at)} · Last used {fmt(key.last_used_at)}
                </p>
              </div>
              <button
                type="button"
                aria-label={`Remove ${key.label}`}
                onClick={() => run(deletePasskey, key.id)}
                className="rounded p-1.5 text-neutral-500 hover:bg-neutral-200 hover:text-red-600 dark:hover:bg-neutral-700"
              >
                <Trash2 size={15} />
              </button>
            </li>
          ))}
          {passkeys.data?.length === 1 && (
            <li className="flex items-center gap-2 text-xs text-amber-700 dark:text-amber-400">
              <AlertTriangle size={13} />
              This is your only passkey. It cannot be removed until you add another.
            </li>
          )}
        </ul>

        <Button
          busy={isAddingPasskey}
          className="self-start"
          onClick={handleAddPasskey}
        >
          <KeyRound size={14} />
          Add a passkey
        </Button>
      </Section>

      <Section
        title="Matrix accounts"
        hint="To link another Matrix account, send !link to the bot in a direct message from it."
      >
        <ul className="flex flex-col gap-2">
          {links.data?.map((identity, idx) => (
            <li
              key={identity.user_id}
              className="flex items-center gap-3 rounded-lg bg-neutral-50 px-3 py-2 dark:bg-neutral-800"
            >
              <Link2 size={16} className="shrink-0 text-neutral-500" />
              <div className="flex items-center gap-2 min-w-0 flex-1">
                <span className="truncate text-sm text-neutral-900 dark:text-neutral-100 font-mono">
                  {identity.user_id}
                </span>
                {idx === 0 && (
                  <span className="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium bg-emerald-100 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300">
                    primary identity
                  </span>
                )}
              </div>
              <button
                type="button"
                aria-label={`Unlink ${identity.user_id}`}
                onClick={() => run(unlink, identity.user_id)}
                className="rounded p-1.5 text-neutral-500 hover:bg-neutral-200 hover:text-red-600 dark:hover:bg-neutral-700"
              >
                <Trash2 size={15} />
              </button>
            </li>
          ))}
          {links.data?.length === 0 && (
            <li className="text-xs text-neutral-500 dark:text-neutral-400">
              No Matrix account linked yet.
            </li>
          )}
        </ul>
      </Section>

      <Section title="Signed-in devices">
        <ul className="flex flex-col gap-2">
          {sessions.data?.map((session) => {
            const device = parseDevice(session.user_agent);
            const DeviceIcon =
              device.type === 'mobile'
                ? Smartphone
                : device.type === 'tablet'
                ? Tablet
                : Monitor;

            return (
              <li
                key={session.id}
                className="flex items-center gap-3 rounded-lg bg-neutral-50 px-3 py-2.5 dark:bg-neutral-800"
              >
                <DeviceIcon size={16} className="shrink-0 text-neutral-500 dark:text-neutral-400" />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span
                      className="truncate text-sm font-medium text-neutral-900 dark:text-neutral-100"
                      title={session.user_agent || undefined}
                    >
                      {device.name}
                    </span>
                    {session.current && (
                      <span className="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium bg-emerald-100 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300">
                        this device
                      </span>
                    )}
                  </div>
                  <p className="text-xs text-neutral-500 dark:text-neutral-400">
                    Last active {fmt(session.last_seen_at)}
                  </p>
                </div>
                {!session.current && (
                  <button
                    type="button"
                    aria-label={`Sign out ${device.name}`}
                    title="Sign out this device"
                    onClick={() => run(revokeSession, session.id)}
                    className="rounded p-1.5 text-neutral-500 hover:bg-neutral-200 hover:text-red-600 dark:hover:bg-neutral-700"
                  >
                    <LogOut size={15} />
                  </button>
                )}
              </li>
            );
          })}
        </ul>
        {(sessions.data?.length ?? 0) > 1 && (
          <Button
            tone="danger"
            busy={revokeOthers.isPending}
            className="self-start"
            onClick={() => run(revokeOthers, undefined as void)}
          >
            Sign out other devices
          </Button>
        )}
      </Section>

      <Section
        title="Recovery codes"
        hint="Single-use codes that get you back in if you lose every passkey. Store them somewhere other than this device."
      >
        {freshCodes ? (
          <div className="flex flex-col gap-2 rounded-lg border border-neutral-300 p-3 dark:border-neutral-700">
            <p className="text-xs font-medium text-amber-700 dark:text-amber-400">
              Save these now — they are not shown again.
            </p>
            <ul className="grid grid-cols-2 gap-1 font-mono text-sm text-neutral-900 dark:text-neutral-100">
              {freshCodes.map((code) => (
                <li key={code} className="select-all">
                  {code}
                </li>
              ))}
            </ul>
            <button
              type="button"
              onClick={() => handleCopy(freshCodes.join('\n'), 'recovery')}
              className="inline-flex items-center gap-1.5 self-start text-xs font-medium text-neutral-600 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-neutral-200 transition-colors"
            >
              {copiedTarget === 'recovery' ? (
                <>
                  <Check size={12} className="text-emerald-600 dark:text-emerald-400" />
                  <span className="text-emerald-600 dark:text-emerald-400 font-semibold">Copied!</span>
                </>
              ) : (
                <>
                  <Copy size={12} />
                  <span>Copy all</span>
                </>
              )}
            </button>
            <Button className="self-start" onClick={() => setFreshCodes(null)}>
              I have saved them
            </Button>
          </div>
        ) : (
          <div className="flex items-center gap-4">
            <p className="text-sm text-neutral-600 dark:text-neutral-400">
              {recovery.data?.codes_remaining ?? 0} unused
            </p>
            <Button
              busy={regenerate.isPending}
              onClick={() => {
                setError(null);
                regenerate
                  .mutateAsync(undefined as void)
                  .then((res: any) => setFreshCodes(res.codes))
                  .catch((err) => setError(describePasskeyError(err)));
              }}
            >
              Generate new codes
            </Button>
          </div>
        )}
      </Section>

      <Section title="Recent activity">
        <ul className="flex flex-col gap-1.5">
          {events.data?.slice(0, 15).map((event) => (
            <li key={event.id} className="flex items-baseline gap-3 text-sm">
              <span
                className={
                  isAlarming(event.kind)
                    ? 'font-medium text-red-600 dark:text-red-400'
                    : 'text-neutral-800 dark:text-neutral-200'
                }
              >
                {describeEvent(event.kind)}
              </span>
              {event.detail && (
                <span className="truncate text-xs text-neutral-500 dark:text-neutral-400">
                  {event.detail}
                </span>
              )}
              <span className="ml-auto shrink-0 text-xs text-neutral-400">
                {fmt(event.created_at)}
              </span>
            </li>
          ))}
          {events.data?.length === 0 && (
            <li className="text-xs text-neutral-500 dark:text-neutral-400">Nothing yet.</li>
          )}
        </ul>
      </Section>
    </div>
  );
};
