import type { components } from '../../types/api';

export type Bookmark = components['schemas']['Bookmark'];
export type BookmarkStats = components['schemas']['BookmarkStats'];
export type CreateBookmarkInput = components['schemas']['CreateBookmarkInput'];
export type UpdateBookmarkInput = components['schemas']['UpdateBookmarkInput'];
export type DomainCount = components['schemas']['DomainCount'];

export type BookmarkFilterTab = 'unread' | 'favorites' | 'archive' | 'all';
