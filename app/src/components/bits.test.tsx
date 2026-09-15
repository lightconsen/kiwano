// The two hand-rolled marks every row can carry: the usage ring and the 7-day
// sparkline.
//
// They are pure SVG with no chart library behind them, which means the geometry
// is this code's own arithmetic — and the failure modes are silent ones: a NaN
// in an attribute is a mark that simply does not appear, and nobody reading a
// provider row can tell "no data" from "broken math".
import { render, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Ring, Sparkline } from "./bits";

/** The painted arc: the track is the `--surface2` circle, the value is the one
    drawn in the colour handed in. */
function arc(container: HTMLElement): SVGCircleElement {
  const el = container.querySelector<SVGCircleElement>('circle[stroke="#123456"]');
  expect(el, "the ring draws a value arc").not.toBeNull();
  return el!;
}

describe("the usage ring", () => {
  // 2πr with the component's r=12, to one decimal: the dash pattern is a share
  // of this, so every case below is a fraction of it.
  const C = 75.4;

  it("draws the share of the circumference it was given", () => {
    const half = render(<Ring pct={50} color="#123456" />);
    expect(arc(half.container)).toHaveAttribute("stroke-dasharray", `37.7 ${C}`);

    const all = render(<Ring pct={100} color="#123456" />);
    expect(arc(all.container)).toHaveAttribute("stroke-dasharray", `${C} ${C}`);
  });

  it("draws nothing for an empty ring", () => {
    const { container, getByText } = render(<Ring pct={0} color="#123456" />);
    expect(arc(container)).toHaveAttribute("stroke-dasharray", `0 ${C}`);
    expect(getByText("0%")).toBeInTheDocument();
  });

  it("clamps what cannot be drawn instead of wrapping the ring", () => {
    // A provider can be over its limit (the ring is the alarm) but the circle
    // cannot be more than full.
    const over = render(<Ring pct={143} color="#123456" />);
    expect(arc(over.container)).toHaveAttribute("stroke-dasharray", `${C} ${C}`);
    expect(over.getByText("100%")).toBeInTheDocument();

    const under = render(<Ring pct={-5} color="#123456" />);
    expect(arc(under.container)).toHaveAttribute("stroke-dasharray", `0 ${C}`);
  });

  it("treats an unmeasurable share as zero rather than as NaN", () => {
    // quota.limit = 0 reaches here as 0/0; `stroke-dasharray="NaN …"` is not a
    // drawing the browser skips quietly — it is an invalid attribute.
    for (const pct of [NaN, Infinity, -Infinity]) {
      const { container } = render(<Ring pct={pct} color="#123456" />);
      expect(arc(container)).toHaveAttribute("stroke-dasharray", `0 ${C}`);
      // Scoped to this render: the loop leaves three rings in the document.
      expect(within(container).getByText("0%")).toBeInTheDocument();
    }
  });
});

describe("the sparkline", () => {
  function polyline(container: HTMLElement): SVGPolylineElement {
    const el = container.querySelector("polyline");
    expect(el, "the sparkline draws a polyline").not.toBeNull();
    return el!;
  }

  it("spreads the samples across the viewBox", () => {
    const { container } = render(<Sparkline points={[2, 4, 6]} />);
    expect(polyline(container)).toHaveAttribute("points", "0,2 40,4 80,6");
  });

  it("draws a single sample as a dot, because a polyline cannot", () => {
    // One day of use is a real shape (the backend sends a one-element series)
    // and `(0 * 80) / (1 - 1)` is 0/0, so the sample goes at the start. A
    // polyline through one point paints *nothing*: the slot sat empty, reading
    // as "no data" exactly where the data says "one day".
    const { container } = render(<Sparkline points={[3]} />);
    expect(container.querySelector("polyline")).toBeNull();
    const dot = container.querySelector("circle")!;
    expect(dot).toHaveAttribute("cy", "3");
    // Off the left edge: half a dot at x(0) = 0 would fall outside the viewBox.
    expect(dot).toHaveAttribute("cx", "2");
    expect(container.innerHTML).not.toContain("NaN");
  });

  it("renders nothing, rather than NaN, for no samples at all", () => {
    const { container } = render(<Sparkline points={[]} />);
    expect(container.querySelector("polyline")).toBeNull();
    expect(container.querySelector("circle")).toBeNull();
    expect(container.innerHTML).not.toContain("NaN");
  });
});
