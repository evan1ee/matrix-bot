import type { components } from '../../types/api';

export type Footprint = components['schemas']['Footprint'];

// Frontend-only name; server schema is CreateFootprintRequest.
export type CreateFootprintInput = components['schemas']['CreateFootprintRequest'];

// Frontend-only name; server schema is UpdateFootprintRequest.
export type UpdateFootprintInput = components['schemas']['UpdateFootprintRequest'];

export type FootprintCurrencySpend = components['schemas']['FootprintCurrencySpend'];

// Frontend-only name; server schema is FootprintsSummary.
export type FootprintsSummaryStats = components['schemas']['FootprintsSummary'];

// Frontend-only: shape of a Nominatim (OpenStreetMap) geocoding API result.
// No server equivalent — this is an external third-party API response.
export interface NominatimSearchResult {
  place_id: number;
  lat: string;
  lon: string;
  display_name: string;
  address?: {
    city?: string;
    town?: string;
    village?: string;
    state?: string;
    country?: string;
  };
}
