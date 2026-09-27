import { SearchableSelect } from "../../components/SearchableSelect";

const LOCAL_ZONE = Intl.DateTimeFormat().resolvedOptions().timeZone;
const CAN_LIST_TIME_ZONES = typeof Intl.supportedValuesOf === "function";
const TIME_ZONES = Array.from(
  new Set([
    "UTC",
    LOCAL_ZONE,
    ...(CAN_LIST_TIME_ZONES ? Intl.supportedValuesOf("timeZone") : []),
  ]),
).sort();

function zoneLabel(zone: string) {
  return zone === "browser"
    ? `Local time (${LOCAL_ZONE.replaceAll("_", " ")})`
    : zone.replaceAll("_", " ");
}

export function TimeZoneSelect({
  value,
  onChange,
}: {
  value: string;
  onChange: (zone: string) => void;
}) {
  const zones = ["browser", ...TIME_ZONES];
  if (!zones.includes(value)) zones.push(value);
  return (
    <SearchableSelect
      value={value}
      onChange={onChange}
      options={zones.map((zone) => ({ value: zone, label: zoneLabel(zone) }))}
      label="Time zone"
      searchLabel="Search time zones"
      placeholder="Search city or time zone"
      emptyMessage="No matching time zones."
      describedBy="profile-time-zone-help"
      hint={
        !CAN_LIST_TIME_ZONES
          ? "This browser can list only UTC, local time, and your saved time zone."
          : undefined
      }
    />
  );
}
