import React, { useMemo } from 'react';
import { marked } from 'marked';
import DOMPurify from 'dompurify';

interface MarkdownRendererProps {
  content: string;
  className?: string;
}

export const MarkdownRenderer: React.FC<MarkdownRendererProps> = ({ content, className = '' }) => {
  const html = useMemo(() => {
    if (!content) return '';
    try {
      const raw = marked.parse(content, {
        gfm: true,
        breaks: true,
      }) as string;
      return DOMPurify.sanitize(raw);
    } catch {
      return content;
    }
  }, [content]);

  return (
    <div
      className={`markdown-body break-words font-sans text-sm leading-relaxed ${className}`}
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
};
