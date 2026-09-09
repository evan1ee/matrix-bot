import React, { useState, useEffect } from 'react';
import { fetchApi } from '../../core/apiClient';
import { Footprint, CreateFootprintInput, NominatimSearchResult } from './types';
import { X, MapPin, Calendar, Star, DollarSign, Search, Loader2, Compass } from 'lucide-react';
import { DatePicker } from '../../components/DatePicker';

interface FootprintModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSave: (data: CreateFootprintInput) => Promise<void>;
  initialData?: Footprint | null;
  initialCoordinates?: { lat: number; lon: number } | null;
}

const CATEGORIES = [
  { id: 'sightseeing', label: 'Sightseeing' },
  { id: 'food', label: 'Food & Dining' },
  { id: 'lodging', label: 'Hotel & Lodging' },
  { id: 'nature', label: 'Nature & Parks' },
  { id: 'roadtrip', label: 'Road Trip' },
  { id: 'work', label: 'Work & Conference' },
];

export const FootprintModal: React.FC<FootprintModalProps> = ({
  isOpen,
  onClose,
  onSave,
  initialData,
  initialCoordinates,
}) => {
  const [title, setTitle] = useState('');
  const [latitude, setLatitude] = useState(0);
  const [longitude, setLongitude] = useState(0);
  const [country, setCountry] = useState('');
  const [city, setCity] = useState('');
  const [address, setAddress] = useState('');
  const [arrivalDate, setArrivalDate] = useState('');
  const [departureDate, setDepartureDate] = useState('');
  const [rating, setRating] = useState(5);
  const [category, setCategory] = useState('sightseeing');
  const [spendAmount, setSpendAmount] = useState('');
  const [spendCurrency, setSpendCurrency] = useState('USD');
  const [note, setNote] = useState('');

  // Geocoding Search
  const [searchQuery, setSearchQuery] = useState('');
  const [isSearching, setIsSearching] = useState(false);
  const [searchResults, setSearchResults] = useState<NominatimSearchResult[]>([]);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  // Reset or populate fields
  useEffect(() => {
    if (initialData) {
      setTitle(initialData.title);
      setLatitude(initialData.latitude);
      setLongitude(initialData.longitude);
      setCountry(initialData.country || '');
      setCity(initialData.city || '');
      setAddress(initialData.address || '');
      setArrivalDate(initialData.arrival_date);
      setDepartureDate(initialData.departure_date || '');
      setRating(initialData.rating);
      setCategory(initialData.category || 'sightseeing');
      setSpendAmount(
        initialData.spend_amount_minor > 0
          ? (initialData.spend_amount_minor / Math.pow(10, initialData.spend_currency_exponent)).toFixed(2)
          : ''
      );
      setSpendCurrency(initialData.spend_currency);
      setNote(initialData.note || '');
      setErrorMsg(null);
    } else {
      const today = new Date().toISOString().split('T')[0];
      setTitle('');
      setLatitude(initialCoordinates?.lat || 0);
      setLongitude(initialCoordinates?.lon || 0);
      setCountry('');
      setCity('');
      setAddress('');
      setArrivalDate(today);
      setDepartureDate('');
      setRating(5);
      setCategory('sightseeing');
      setSpendAmount('');
      setSpendCurrency('USD');
      setNote('');
      setSearchQuery('');
      setSearchResults([]);
      setErrorMsg(null);

      if (initialCoordinates) {
        reverseGeocode(initialCoordinates.lat, initialCoordinates.lon);
      }
    }
  }, [initialData, initialCoordinates, isOpen]);

  const reverseGeocode = async (lat: number, lon: number) => {
    try {
      setIsSearching(true);
      const res = await fetchApi<any>(`/footprints/geocode?lat=${lat}&lon=${lon}`);
      if (res && res.display_name) {
        setAddress(res.display_name);
        const namePart = res.name || res.display_name.split(',')[0] || '';
        setTitle(namePart);
        if (res.address) {
          setCity(res.address.city || res.address.town || res.address.village || res.address.state || '');
          setCountry(res.address.country || '');
        }
      }
    } catch {
      // Ignore geocode error
    } finally {
      setIsSearching(false);
    }
  };

  const handleSearchPlaces = async () => {
    if (!searchQuery.trim()) return;
    try {
      setIsSearching(true);
      const results = await fetchApi<NominatimSearchResult[]>(
        `/footprints/geocode?query=${encodeURIComponent(searchQuery)}`
      );
      setSearchResults(results || []);
    } catch {
      setSearchResults([]);
    } finally {
      setIsSearching(false);
    }
  };

  const selectSearchResult = (item: NominatimSearchResult) => {
    setTitle(item.display_name.split(',')[0]);
    setAddress(item.display_name);
    setLatitude(parseFloat(item.lat));
    setLongitude(parseFloat(item.lon));
    if (item.address) {
      setCity(item.address.city || item.address.town || item.address.village || item.address.state || '');
      setCountry(item.address.country || '');
    }
    setSearchResults([]);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!title.trim()) {
      setErrorMsg('Place name is required.');
      return;
    }
    if (!arrivalDate) {
      setErrorMsg('Arrival date is required.');
      return;
    }

    try {
      setIsSubmitting(true);
      setErrorMsg(null);

      await onSave({
        title: title.trim(),
        latitude,
        longitude,
        country: country.trim() || undefined,
        city: city.trim() || undefined,
        address: address.trim() || undefined,
        arrival_date: arrivalDate,
        departure_date: departureDate || undefined,
        spend_amount: spendAmount.trim() || undefined,
        spend_currency: spendCurrency,
        rating,
        category,
        note: note.trim() || undefined,
      });

      onClose();
    } catch (err: any) {
      setErrorMsg(err.message || 'Failed to save footprint');
    } finally {
      setIsSubmitting(false);
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 bg-black/60 backdrop-blur-xs flex items-center justify-center p-4">
      <div className="w-full max-w-xl rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 shadow-xl overflow-hidden p-6 space-y-4">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <div className="flex items-center gap-2">
            <Compass className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
            <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">
              {initialData ? 'Edit Visited Place' : 'Record Visited Place'}
            </h3>
          </div>
          <button
            onClick={onClose}
            className="p-1 rounded text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 transition cursor-pointer"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {errorMsg && (
          <div className="p-3 rounded bg-red-100 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs font-mono">
            {errorMsg}
          </div>
        )}

        <form onSubmit={handleSubmit} className="space-y-3.5">
          {/* Autocomplete Search Bar */}
          <div>
            <span className="text-[10px] uppercase font-mono text-neutral-500 tracking-wider block mb-1">
              Search Place / Landmark
            </span>
            <div className="flex gap-2">
              <div className="relative flex-1">
                <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-neutral-400" />
                <input
                  type="text"
                  placeholder="e.g. Eiffel Tower, Kyoto Fushimi Inari..."
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      handleSearchPlaces();
                    }
                  }}
                  className="w-full h-9 pl-9 pr-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500 font-sans"
                />
              </div>
              <button
                type="button"
                onClick={handleSearchPlaces}
                disabled={isSearching}
                className="h-9 px-3.5 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 flex items-center gap-1.5 cursor-pointer"
              >
                {isSearching ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : null}
                <span>Search</span>
              </button>
            </div>

            {/* Dropdown Results */}
            {searchResults.length > 0 && (
              <div className="mt-1 rounded border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 shadow-lg divide-y divide-neutral-100 dark:divide-neutral-800 max-h-40 overflow-y-auto">
                {searchResults.map((item) => (
                  <button
                    key={item.place_id}
                    type="button"
                    onClick={() => selectSearchResult(item)}
                    className="w-full text-left px-3 py-2 text-xs hover:bg-neutral-100 dark:hover:bg-neutral-800 transition flex items-start gap-2 cursor-pointer"
                  >
                    <MapPin className="w-3.5 h-3.5 text-neutral-400 shrink-0 mt-0.5" />
                    <span className="truncate text-neutral-800 dark:text-neutral-200">{item.display_name}</span>
                  </button>
                ))}
              </div>
            )}
          </div>

          {/* Place Name & Category */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <div>
              <label className="text-[10px] uppercase font-mono text-neutral-500 tracking-wider block mb-1">
                Place Name *
              </label>
              <input
                type="text"
                required
                placeholder="e.g. Grand Canyon"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                className="w-full h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500 font-sans"
              />
            </div>

            <div>
              <label className="text-[10px] uppercase font-mono text-neutral-500 tracking-wider block mb-1">
                Category
              </label>
              <select
                value={category}
                onChange={(e) => setCategory(e.target.value)}
                className="w-full h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-sans cursor-pointer"
              >
                {CATEGORIES.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.label}
                  </option>
                ))}
              </select>
            </div>
          </div>

          {/* City & Country */}
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="text-[10px] uppercase font-mono text-neutral-500 tracking-wider block mb-1">
                City
              </label>
              <input
                type="text"
                placeholder="e.g. Paris"
                value={city}
                onChange={(e) => setCity(e.target.value)}
                className="w-full h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500 font-sans"
              />
            </div>
            <div>
              <label className="text-[10px] uppercase font-mono text-neutral-500 tracking-wider block mb-1">
                Country
              </label>
              <input
                type="text"
                placeholder="e.g. France"
                value={country}
                onChange={(e) => setCountry(e.target.value)}
                className="w-full h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500 font-sans"
              />
            </div>
          </div>

          {/* Dates */}
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="text-[10px] uppercase font-mono text-neutral-500 tracking-wider block mb-1">
                Arrival Date *
              </label>
              <DatePicker
                required
                value={arrivalDate}
                onChange={setArrivalDate}
                placeholder="Arrival date"
              />
            </div>
            <div>
              <label className="text-[10px] uppercase font-mono text-neutral-500 tracking-wider block mb-1">
                Departure Date (Optional)
              </label>
              <DatePicker
                min={arrivalDate}
                value={departureDate}
                onChange={setDepartureDate}
                placeholder="Departure date"
              />
            </div>
          </div>

          {/* Rating & Spend */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <div>
              <label className="text-[10px] uppercase font-mono text-neutral-500 tracking-wider block mb-1">
                Rating
              </label>
              <div className="flex items-center gap-1 pt-1">
                {[1, 2, 3, 4, 5].map((s) => (
                  <button
                    key={s}
                    type="button"
                    onClick={() => setRating(s)}
                    className="p-1 text-neutral-300 dark:text-neutral-700 hover:text-amber-500 dark:hover:text-amber-400 transition-colors duration-150 cursor-pointer"
                  >
                    <Star
                      className={`w-5 h-5 ${
                        s <= rating
                          ? 'text-amber-500 fill-amber-500 dark:text-amber-400 dark:fill-amber-400'
                          : 'text-neutral-300 dark:text-neutral-700'
                      }`}
                    />
                  </button>
                ))}
              </div>
            </div>

            <div>
              <label className="text-[10px] uppercase font-mono text-neutral-500 tracking-wider block mb-1">
                Total Spend
              </label>
              <div className="flex gap-2">
                <input
                  type="number"
                  step="0.01"
                  min="0"
                  placeholder="0.00"
                  value={spendAmount}
                  onChange={(e) => setSpendAmount(e.target.value)}
                  className="flex-1 h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500 font-mono"
                />
                <select
                  value={spendCurrency}
                  onChange={(e) => setSpendCurrency(e.target.value)}
                  className="w-24 h-9 px-2 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs font-mono text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 cursor-pointer"
                >
                  <option value="USD">USD</option>
                  <option value="EUR">EUR</option>
                  <option value="GBP">GBP</option>
                  <option value="JPY">JPY</option>
                  <option value="CNY">CNY</option>
                  <option value="CAD">CAD</option>
                  <option value="AUD">AUD</option>
                </select>
              </div>
            </div>
          </div>

          {/* Notes */}
          <div>
            <label className="text-[10px] uppercase font-mono text-neutral-500 tracking-wider block mb-1">
              Memories &amp; Travel Notes
            </label>
            <textarea
              rows={3}
              placeholder="What made this place special? Highlights, favorite spots, memories..."
              value={note}
              onChange={(e) => setNote(e.target.value)}
              className="w-full px-3 py-2 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500 font-sans resize-none"
            />
          </div>

          {/* Footer Actions */}
          <div className="flex items-center justify-end gap-2 pt-2 border-t border-neutral-200 dark:border-neutral-800">
            <button
              type="button"
              onClick={onClose}
              className="h-9 px-3.5 rounded-md border border-neutral-300 dark:border-neutral-700 text-xs font-medium text-neutral-700 dark:text-neutral-300 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={isSubmitting}
              className="h-9 px-4 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 flex items-center gap-1.5 cursor-pointer"
            >
              {isSubmitting ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : null}
              <span>Save Footprint</span>
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
