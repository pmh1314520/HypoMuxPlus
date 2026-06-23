import { SpeedHero } from "./SpeedHero";
import { AdapterTable } from "./AdapterTable";
import { Console } from "./Console";
import type { AdapterInfo, NicTelemetry, TelemetryPayload } from "../lib/api";

interface Props {
  telemetry: TelemetryPayload | null;
  history: number[];
  peak: number;
  uptime: number;
  running: boolean;
  busy: boolean;
  canBoost: boolean;
  onBoost: () => void;

  adapters: AdapterInfo[];
  selected: Set<number>;
  toggle: (i: number) => void;
  selectAll: () => void;
  deselectAll: () => void;
  refresh: () => void;
  perNic: Record<string, NicTelemetry>;
  loading: boolean;

  logs: string[];
  clearLogs: () => void;
}

export function Dashboard(props: Props) {
  return (
    <div className="h-full flex flex-col gap-4">
      <SpeedHero
        telemetry={props.telemetry}
        history={props.history}
        peak={props.peak}
        uptime={props.uptime}
        running={props.running}
        busy={props.busy}
        canBoost={props.canBoost}
        onBoost={props.onBoost}
      />
      <div className="flex-1 grid gap-4 min-h-0" style={{ gridTemplateColumns: "1.35fr 1fr" }}>
        <AdapterTable
          adapters={props.adapters}
          selected={props.selected}
          toggle={props.toggle}
          selectAll={props.selectAll}
          deselectAll={props.deselectAll}
          refresh={props.refresh}
          perNic={props.perNic}
          running={props.running}
          loading={props.loading}
        />
        <Console logs={props.logs} clear={props.clearLogs} />
      </div>
    </div>
  );
}
