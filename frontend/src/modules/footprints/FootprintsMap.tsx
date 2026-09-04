import React, { useEffect, useRef, useState } from 'react';
import L from 'leaflet';
import 'leaflet/dist/leaflet.css';
import { Footprint } from './types';
import { Navigation, Compass } from 'lucide-react';

interface FootprintsMapProps {
  footprints: Footprint[];
  selectedFootprint: Footprint | null;
  onSelectFootprint: (fp: Footprint) => void;
  onMapClick: (lat: number, lon: number) => void;
  pendingPin: { lat: number; lon: number } | null;
}

export const FootprintsMap: React.FC<FootprintsMapProps> = ({
  footprints,
  selectedFootprint,
  onSelectFootprint,
  onMapClick,
  pendingPin,
}) => {
  const mapContainerRef = useRef<HTMLDivElement>(null);
  const [map, setMap] = useState<L.Map | null>(null);
  const markersRef = useRef<{ [id: number]: L.Marker }>({});
  const pendingMarkerRef = useRef<L.Marker | null>(null);

  const onMapClickRef = useRef(onMapClick);
  onMapClickRef.current = onMapClick;

  // Initialize Leaflet map
  useEffect(() => {
    if (!mapContainerRef.current) return;

    const leafletMap = L.map(mapContainerRef.current, {
      center: [25.0, 15.0],
      zoom: 2,
      minZoom: 2,
      maxZoom: 18,
      worldCopyJump: true,
    });

    // Standard OpenStreetMap tiles (free, reliable, zero watermarks)
    L.tileLayer('https://tile.openstreetmap.org/{z}/{x}/{y}.png', {
      attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
      maxZoom: 19,
    }).addTo(leafletMap);

    leafletMap.on('click', (e: L.LeafletMouseEvent) => {
      onMapClickRef.current(e.latlng.lat, e.latlng.lng);
    });

    setMap(leafletMap);

    // Initial size invalidation
    const timer = setTimeout(() => {
      leafletMap.invalidateSize();
    }, 250);

    // ResizeObserver ensures map size is always accurate and never renders outside container
    let resizeObserver: ResizeObserver | null = null;
    if (typeof ResizeObserver !== 'undefined' && mapContainerRef.current) {
      resizeObserver = new ResizeObserver(() => {
        leafletMap.invalidateSize();
      });
      resizeObserver.observe(mapContainerRef.current);
    }

    return () => {
      clearTimeout(timer);
      if (resizeObserver) {
        resizeObserver.disconnect();
      }
      leafletMap.remove();
      setMap(null);
    };
  }, []);

  // Update existing star markers
  useEffect(() => {
    if (!map) return;

    // Clear removed markers
    Object.keys(markersRef.current).forEach((idStr) => {
      const id = Number(idStr);
      if (!footprints.some((f) => f.id === id)) {
        markersRef.current[id].remove();
        delete markersRef.current[id];
      }
    });

    // Add or update markers
    footprints.forEach((fp) => {
      const isSelected = selectedFootprint?.id === fp.id;
      const bg = isSelected ? '#d97706' : '#f59e0b';
      const scale = isSelected ? '1.2' : '1';

      const customIcon = L.divIcon({
        className: 'footprint-star-div-icon',
        html: `
          <div aria-label="${fp.title}" style="position: relative; width: 32px; height: 32px; display: flex; align-items: center; justify-content: center; cursor: pointer;">
            <div style="width: 30px; height: 30px; border-radius: 50%; background: ${bg}; border: 2px solid #ffffff; box-shadow: 0 2px 8px rgba(0,0,0,0.35); display: flex; align-items: center; justify-content: center; color: #ffffff; font-size: 15px; font-weight: bold; transform: scale(${scale}); transition: transform 0.15s ease;">
              ★
            </div>
            <div style="position: absolute; bottom: -20px; left: 50%; transform: translateX(-50%); background: #171717; color: #ffffff; padding: 1px 6px; border-radius: 4px; font-size: 10px; font-family: monospace; font-weight: 500; white-space: nowrap; max-width: 140px; overflow: hidden; text-overflow: ellipsis; border: 1px solid #333333; box-shadow: 0 2px 4px rgba(0,0,0,0.4); pointer-events: none;">
              ${fp.title}
            </div>
          </div>
        `,
        iconSize: [32, 32],
        iconAnchor: [16, 16],
        popupAnchor: [0, -18],
      });

      if (markersRef.current[fp.id]) {
        markersRef.current[fp.id].setIcon(customIcon);
        markersRef.current[fp.id].setLatLng([fp.latitude, fp.longitude]);
      } else {
        const marker = L.marker([fp.latitude, fp.longitude], { icon: customIcon }).addTo(map);

        marker.on('click', (e) => {
          L.DomEvent.stopPropagation(e);
          onSelectFootprint(fp);
        });

        markersRef.current[fp.id] = marker;
      }
    });

    // If footprints exist and none is focused, fit map bounds
    if (footprints.length > 0 && !selectedFootprint) {
      const bounds = L.latLngBounds(footprints.map((f) => [f.latitude, f.longitude]));
      map.fitBounds(bounds, { padding: [40, 40], maxZoom: 8 });
    }
  }, [map, footprints, selectedFootprint, onSelectFootprint]);

  // Handle pending pin drop
  useEffect(() => {
    if (!map) return;

    if (pendingMarkerRef.current) {
      pendingMarkerRef.current.remove();
      pendingMarkerRef.current = null;
    }

    if (pendingPin) {
      const pinIcon = L.divIcon({
        className: 'pending-pin-div-icon',
        html: `
          <div style="width: 32px; height: 32px; border-radius: 50%; background: #10b981; border: 2px solid #ffffff; box-shadow: 0 2px 8px rgba(16, 185, 129, 0.6); display: flex; align-items: center; justify-content: center; color: #ffffff; font-size: 18px; font-weight: bold;">
            +
          </div>
        `,
        iconSize: [32, 32],
        iconAnchor: [16, 16],
      });

      pendingMarkerRef.current = L.marker([pendingPin.lat, pendingPin.lon], { icon: pinIcon }).addTo(map);
    }
  }, [map, pendingPin]);

  // Smoothly center when selectedFootprint changes
  useEffect(() => {
    if (!map || !selectedFootprint) return;

    map.flyTo([selectedFootprint.latitude, selectedFootprint.longitude], Math.max(map.getZoom(), 8), {
      duration: 1.2,
      easeLinearity: 0.25,
    });
  }, [map, selectedFootprint]);

  const handleResetView = () => {
    if (!map) return;

    if (footprints.length > 0) {
      const bounds = L.latLngBounds(footprints.map((f) => [f.latitude, f.longitude]));
      map.fitBounds(bounds, { padding: [40, 40], maxZoom: 12 });
    } else {
      map.flyTo([25.0, 15.0], 2);
    }
  };

  return (
    <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4 overflow-hidden">
      {/* Top Header */}
      <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3 shrink-0">
        <div className="flex items-center gap-2">
          <Compass className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
          <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">Interactive World Map</h3>
        </div>
        <div className="flex items-center gap-3">
          <div className="hidden sm:flex items-center gap-1.5 text-xs text-neutral-500 font-mono">
            <span className="w-1.5 h-1.5 rounded-full bg-amber-400" />
            <span>Click anywhere on map to mark place</span>
          </div>
          <span className="text-xs font-mono text-neutral-400 dark:text-neutral-500">OpenStreetMap</span>
        </div>
      </div>

      {/* Map Body: strict containment, isolation, and overflow clipping */}
      <div className="relative w-full h-[380px] sm:h-[440px] rounded-md border border-neutral-200 dark:border-neutral-800 overflow-hidden bg-neutral-100 dark:bg-neutral-900 isolate [contain:paint]">
        <div
          ref={mapContainerRef}
          className="w-full h-full rounded-md overflow-hidden"
          style={{ zIndex: 1, minHeight: '100%', minWidth: '100%' }}
        />

        {/* Floating Controls */}
        <div className="absolute top-3 right-3 z-20 flex flex-col gap-1.5">
          <button
            onClick={handleResetView}
            title="Fit All Visited Places"
            className="p-1.5 bg-white dark:bg-neutral-900 hover:bg-neutral-100 dark:hover:bg-neutral-800 text-neutral-700 dark:text-neutral-200 rounded border border-neutral-300 dark:border-neutral-700 shadow-sm transition cursor-pointer"
          >
            <Navigation className="w-3.5 h-3.5" />
          </button>
        </div>

        {/* Helper Banner overlay (small screens) */}
        <div className="sm:hidden absolute bottom-3 left-3 z-20 pointer-events-none">
          <div className="px-2.5 py-1 rounded bg-neutral-950/90 text-neutral-200 text-[10px] font-mono border border-neutral-800 shadow-sm flex items-center gap-1.5">
            <span className="w-1.5 h-1.5 rounded-full bg-amber-400" />
            <span>Click anywhere on map to mark place</span>
          </div>
        </div>
      </div>
    </div>
  );
};
