export interface Footprint {
  id: number;
  owner_id: string;
  title: string;
  latitude: number;
  longitude: number;
  country: string | null;
  city: string | null;
  address: string | null;
  arrival_date: string;
  departure_date: string | null;
  stay_duration_days: number;
  spend_amount_minor: number;
  spend_currency: string;
  spend_currency_exponent: number;
  rating: number;
  category: string;
  note: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateFootprintInput {
  title: string;
  latitude: number;
  longitude: number;
  country?: string | null;
  city?: string | null;
  address?: string | null;
  arrival_date: string;
  departure_date?: string | null;
  spend_amount?: string;
  spend_currency?: string;
  rating?: number;
  category?: string;
  note?: string | null;
}

export interface UpdateFootprintInput extends Partial<CreateFootprintInput> {}

export interface FootprintCurrencySpend {
  currency: string;
  currency_exponent: number;
  total_spend_minor: number;
}

export interface FootprintsSummaryStats {
  total_places: number;
  total_countries: number;
  total_days_traveled: number;
  spend_by_currency: FootprintCurrencySpend[];
}

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
