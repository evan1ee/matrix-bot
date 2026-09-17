import React, { useState, useRef, useEffect } from 'react';
import { Mic, Square, Loader2, AlertCircle } from 'lucide-react';

export interface VoiceDictateButtonProps {
  onTranscript: (text: string) => void;
  disabled?: boolean;
  className?: string;
}

export const VoiceDictateButton: React.FC<VoiceDictateButtonProps> = ({
  onTranscript,
  disabled = false,
  className = '',
}) => {
  const [status, setStatus] = useState<'idle' | 'recording' | 'transcribing'>('idle');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const chunksRef = useRef<Blob[]>([]);

  useEffect(() => {
    return () => {
      // Cleanup tracks if component unmounts while recording
      if (streamRef.current) {
        streamRef.current.getTracks().forEach((t) => t.stop());
      }
    };
  }, []);

  const startRecording = async () => {
    setErrorMsg(null);
    if (!navigator.mediaDevices?.getUserMedia) {
      setErrorMsg('Microphone access is not supported by your browser.');
      return;
    }

    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      streamRef.current = stream;
      chunksRef.current = [];

      // Determine supported mime type
      const mimeType = MediaRecorder.isTypeSupported('audio/webm;codecs=opus')
        ? 'audio/webm;codecs=opus'
        : MediaRecorder.isTypeSupported('audio/ogg;codecs=opus')
          ? 'audio/ogg;codecs=opus'
          : '';

      const recorder = mimeType ? new MediaRecorder(stream, { mimeType }) : new MediaRecorder(stream);
      mediaRecorderRef.current = recorder;

      recorder.ondataavailable = (e) => {
        if (e.data && e.data.size > 0) {
          chunksRef.current.push(e.data);
        }
      };

      recorder.onstop = async () => {
        if (streamRef.current) {
          streamRef.current.getTracks().forEach((t) => t.stop());
          streamRef.current = null;
        }

        const audioBlob = new Blob(chunksRef.current, {
          type: recorder.mimeType || 'audio/webm',
        });
        chunksRef.current = [];

        if (audioBlob.size === 0) {
          setStatus('idle');
          return;
        }

        setStatus('transcribing');
        try {
          const formData = new FormData();
          const ext = audioBlob.type.includes('ogg') ? 'ogg' : 'webm';
          formData.append('file', audioBlob, `recording.${ext}`);

          const response = await fetch('/api/v1/voice/transcribe', {
            method: 'POST',
            body: formData,
            credentials: 'include',
          });

          if (!response.ok) {
            const errJson = await response.json().catch(() => ({}));
            throw new Error(errJson.error || `Voice transcription error (${response.status})`);
          }

          const data = await response.json();
          if (data.text && data.text.trim()) {
            onTranscript(data.text.trim());
          }
        } catch (err: any) {
          setErrorMsg(err.message || 'Failed to transcribe audio.');
        } finally {
          setStatus('idle');
        }
      };

      recorder.start(250);
      setStatus('recording');
    } catch (err: any) {
      if (err.name === 'NotAllowedError') {
        setErrorMsg('Microphone permission denied.');
      } else {
        setErrorMsg(err.message || 'Failed to access microphone.');
      }
      setStatus('idle');
    }
  };

  const stopRecording = () => {
    if (mediaRecorderRef.current && status === 'recording') {
      mediaRecorderRef.current.stop();
    }
  };

  const handleClick = (e: React.MouseEvent) => {
    e.preventDefault();
    if (status === 'idle') {
      startRecording();
    } else if (status === 'recording') {
      stopRecording();
    }
  };

  return (
    <div className="relative inline-flex items-center">
      <button
        type="button"
        onClick={handleClick}
        disabled={disabled || status === 'transcribing'}
        title={
          status === 'recording'
            ? 'Stop dictation'
            : status === 'transcribing'
              ? 'Transcribing audio...'
              : 'Dictate voice input'
        }
        aria-label={
          status === 'recording'
            ? 'Stop recording'
            : status === 'transcribing'
              ? 'Transcribing'
              : 'Start voice dictation'
        }
        data-testid="voice-dictate-button"
        className={`p-1.5 rounded transition duration-150 cursor-pointer flex items-center justify-center ${
          status === 'recording'
            ? 'bg-rose-600 text-white hover:bg-rose-700'
            : status === 'transcribing'
              ? 'bg-neutral-200 dark:bg-neutral-800 text-neutral-500 cursor-not-allowed'
              : 'border border-neutral-300 dark:border-neutral-700 text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-100 dark:hover:bg-neutral-800'
        } ${className}`}
      >
        {status === 'recording' ? (
          <Square className="w-3.5 h-3.5 fill-current" />
        ) : status === 'transcribing' ? (
          <Loader2 className="w-3.5 h-3.5 animate-spin" />
        ) : (
          <Mic className="w-3.5 h-3.5" />
        )}
      </button>

      {errorMsg && (
        <div
          title={errorMsg}
          className="absolute -top-8 right-0 flex items-center gap-1 bg-red-100 dark:bg-red-950/80 border border-red-200 dark:border-red-800 text-red-800 dark:text-red-300 text-[10px] font-mono px-2 py-0.5 rounded shadow-sm whitespace-nowrap z-10"
        >
          <AlertCircle className="w-3 h-3 shrink-0" />
          <span className="truncate max-w-[180px]">{errorMsg}</span>
          <button
            type="button"
            onClick={() => setErrorMsg(null)}
            className="ml-1 text-red-600 hover:text-red-800 dark:hover:text-red-200"
          >
            ×
          </button>
        </div>
      )}
    </div>
  );
};