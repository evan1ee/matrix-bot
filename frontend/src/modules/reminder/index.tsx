import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { FrontendModule } from '../../core/types';
import { nowInZone, useSettings } from '../../core/settings';
import { ReminderHistory } from './ReminderHistory';
import { CreateReminderCard } from './components/CreateReminderCard';
import { PendingRemindersCard } from './components/PendingRemindersCard';
import type {
  ReminderItem,
  ReminderListResponse,
  JoinedRoomInfo,
  DurationUnit,
} from './types';
import { formatSenderDisplay } from './types';

export {
  formatSenderDisplay,
  type ReminderItem,
  type ReminderListResponse,
  type JoinedRoomInfo,
};

export const ReminderMainView: React.FC<{ autoFocus?: boolean }> = ({ autoFocus }) => {
  const { timezone } = useSettings();
  const queryClient = useQueryClient();
  const messageInputRef = React.useRef<HTMLInputElement>(null);
  const [message, setMessage] = useState('');
  const [durationValue, setDurationValue] = useState<number>(5);
  const [durationUnit, setDurationUnit] = useState<DurationUnit>('minutes');

  // Custom Time Picker State
  const [hour12, setHour12] = useState('09');
  const [minute, setMinute] = useState('30');
  const [amPm, setAmPm] = useState<'AM' | 'PM'>('AM');

  const [recurrence, setRecurrence] = useState('none');
  const [roomId, setRoomId] = useState('');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [now, setNow] = useState<number>(Date.now());

  React.useEffect(() => {
    const timer = setInterval(() => {
      setNow(Date.now());
    }, 1000);
    return () => clearInterval(timer);
  }, []);

  React.useEffect(() => {
    if (autoFocus) messageInputRef.current?.focus();
  }, [autoFocus]);

  const { data, isLoading } = useQuery<ReminderListResponse>({
    queryKey: ['reminders'],
    queryFn: () => fetchApi<ReminderListResponse>('/reminders?limit=100'),
  });

  const { data: roomsData } = useQuery<JoinedRoomInfo[]>({
    queryKey: ['rooms'],
    queryFn: () => fetchApi<JoinedRoomInfo[]>('/rooms'),
  });

  React.useEffect(() => {
    if (roomsData && roomsData.length > 0 && (!roomId || roomId === 'web' || roomId === 'web_chat')) {
      const defaultRoom = roomsData.find((r) => r.is_direct)?.room_id || roomsData[0].room_id;
      setRoomId(defaultRoom);
    }
  }, [roomsData]);

  const createMutation = useMutation({
    mutationFn: (newReminder: { room_id: string; message: string; seconds: number; recurrence: string }) =>
      fetchApi<ReminderItem>('/reminders', {
        method: 'POST',
        body: JSON.stringify(newReminder),
      }),
    onSuccess: () => {
      setMessage('');
      setErrorMsg(null);
      queryClient.invalidateQueries({ queryKey: ['reminders'] });
    },
    onError: (err: Error) => {
      setErrorMsg(err.message);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: number) =>
      fetchApi(`/reminders/${id}`, {
        method: 'DELETE',
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['reminders'] });
    },
  });

  const getCalculatedSeconds = (): number => {
    switch (durationUnit) {
      case 'seconds':
        return durationValue;
      case 'minutes':
        return durationValue * 60;
      case 'hours':
        return durationValue * 3600;
      case 'days':
        return durationValue * 86400;
      default:
        return durationValue * 60;
    }
  };

  const calculateSecondsForRecurringTime = (): number => {
    let targetHours = parseInt(hour12, 10) || 9;
    const targetMinutes = parseInt(minute, 10) || 0;

    if (amPm === 'PM' && targetHours < 12) {
      targetHours += 12;
    } else if (amPm === 'AM' && targetHours === 12) {
      targetHours = 0;
    }

    // "Daily at 8am" means 8am where the user is, not where their browser is.
    // Both clocks are the same zone's wall clock, so their difference is the
    // real interval to wait.
    const currDate = nowInZone(timezone);
    const target = nowInZone(timezone);
    target.setHours(targetHours, targetMinutes, 0, 0);

    if (target.getTime() <= currDate.getTime()) {
      target.setDate(target.getDate() + 1);
    }

    return Math.max(1, Math.floor((target.getTime() - currDate.getTime()) / 1000));
  };

  const handleCreate = (e: React.FormEvent) => {
    e.preventDefault();
    if (!message.trim()) return;

    const computedSecs =
      recurrence === 'none'
        ? getCalculatedSeconds()
        : calculateSecondsForRecurringTime();

    if (computedSecs <= 0) {
      setErrorMsg('Duration must be greater than 0.');
      return;
    }
    if (message.length > 500) {
      setErrorMsg('Message exceeds 500 character limit.');
      return;
    }
    const targetRoom =
      roomId ||
      (roomsData && roomsData.length > 0
        ? roomsData.find((r) => r.is_direct)?.room_id || roomsData[0].room_id
        : 'web');
    createMutation.mutate({ room_id: targetRoom, message: message.trim(), seconds: computedSecs, recurrence });
  };

  const calculateTimeLeft = (triggerAtStr: string) => {
    const diffMs = new Date(triggerAtStr).getTime() - now;
    if (diffMs <= 0) return 'Triggering...';
    const totalSecs = Math.floor(diffMs / 1000);
    const mins = Math.floor(totalSecs / 60);
    const secs = totalSecs % 60;
    if (mins >= 60) {
      const hours = Math.floor(mins / 60);
      return `in ${hours}h ${mins % 60}m`;
    }
    if (mins > 0) return `in ${mins}m ${secs}s`;
    return `in ${secs}s`;
  };

  const getRoomDisplayName = (id: string) => {
    if (!roomsData) {
      if (id === 'web' || id === 'web_chat') return 'Web';
      return id;
    }
    const match = roomsData.find((r) => r.room_id === id);
    if (match) return match.name;
    if (id === 'web' || id === 'web_chat') {
      const direct = roomsData.find((r) => r.is_direct);
      return direct ? direct.name : 'Web';
    }
    return id;
  };

  const handleReuse = (reusedMessage: string, reusedRecurrence: string) => {
    setMessage(reusedMessage);
    setRecurrence(reusedRecurrence || 'none');
    messageInputRef.current?.focus();
    window.scrollTo({ top: 0, behavior: 'smooth' });
  };

  return (
    <div className="space-y-6">
      {/* Create Reminder Form */}
      <CreateReminderCard
        message={message}
        setMessage={setMessage}
        durationValue={durationValue}
        setDurationValue={setDurationValue}
        durationUnit={durationUnit}
        setDurationUnit={setDurationUnit}
        hour12={hour12}
        setHour12={setHour12}
        minute={minute}
        setMinute={setMinute}
        amPm={amPm}
        setAmPm={setAmPm}
        recurrence={recurrence}
        setRecurrence={setRecurrence}
        roomId={roomId}
        setRoomId={setRoomId}
        roomsData={roomsData}
        errorMsg={errorMsg}
        isPending={createMutation.isPending}
        onSubmit={handleCreate}
        messageInputRef={messageInputRef}
      />

      {/* Reminder List */}
      <PendingRemindersCard
        data={data}
        isLoading={isLoading}
        timezone={timezone}
        roomLabel={getRoomDisplayName}
        calculateTimeLeft={calculateTimeLeft}
        onDelete={(id) => deleteMutation.mutate(id)}
      />

      {/* Reminder History */}
      <ReminderHistory
        timezone={timezone}
        roomLabel={getRoomDisplayName}
        onReuse={handleReuse}
      />
    </div>
  );
};

export const reminderModule: FrontendModule = {
  id: 'reminder',
  name: 'Reminders',
  description: 'Scheduled execution and recurring reminders',
  navTab: { label: 'Reminders' },
  MainView: ReminderMainView,
  handleSseEvent: (event, queryClient) => {
    if (
      event.type === 'ReminderCreated' ||
      event.type === 'ReminderUpdated' ||
      event.type === 'ReminderTriggered' ||
      event.type === 'ReminderCancelled' ||
      event.type === 'ReminderDeleted' ||
      event.type === 'ReminderHistoryCleared'
    ) {
      queryClient.invalidateQueries({ queryKey: ['reminders'] });
      queryClient.invalidateQueries({ queryKey: ['reminders-history'] });
    }
  },
};
