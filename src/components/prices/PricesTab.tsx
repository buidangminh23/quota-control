/**
 * Bảng giá: the official Claude and OpenAI price lists for every model, per processing tier, as tables
 * the way the pages publish them (`PRICES_RETRIEVED_AT`). Wide page tables are cut per column group so
 * they fit the popup. The Vietnamese UI can show đồng at the Vietcombank rate; the source page opens
 * in the browser.
 */
import { useMemo, type CSSProperties } from "react";
import { messagesFor, type Language } from "@/i18n";
import { backend } from "@/lib/backend";
import type { ExchangeRate } from "@/lib/types";
import { formatDongNumber, formatDongPrice, formatPageDollars, formatRate, showsDong } from "@/model/currency";
import { shortTime, type TimeFormat } from "@/model/format";
import { SOURCE_COLORS, VIEW_COLORS } from "@/model/palette";
import { NOT_OFFERED, PRICES_RETRIEVED_AT, priceSections, priceSource, priceText, priceTiers, purePrice, type PriceCell, type PriceTable } from "@/model/prices";
import { PRICE_PROVIDERS, type PriceProvider } from "@/model/settings";
import { useSettings } from "@/state/hooks";
import { updateSettings, useApp } from "@/state/store";
import { ProviderMark } from "../ui/ProviderMark";
import { ExternalIcon, TagIcon } from "../ui/icons";
import { tooltipProps } from "../ui/tooltip";
import { Capsules, ChipHeader, dayText } from "../tokens/parts";

const PROVIDER_BRANDS: Readonly<Record<PriceProvider, { brand: string; color: string; name: string }>> = {
  claude: { brand: "claude", color: SOURCE_COLORS.claude, name: "Claude" },
  openai: { brand: "codex", color: SOURCE_COLORS.codex, name: "OpenAI" },
};

/** How a price cell is written: the currency and, for đồng, the rate. */
interface Money {
  language: Language;
  rate: ExchangeRate | null;
}

function rateLine(rate: ExchangeRate, language: Language, timeFormat: TimeFormat): string {
  const published = new Date(rate.publishedAt);
  const time = Number.isNaN(published.getTime()) ? "" : `${shortTime(published, timeFormat, language)} ${messagesFor(language).format.monthDay(published)}`;
  return messagesFor(language).usage.rateNote(formatRate(rate, language), time, rate.stale);
}

/** A price cell's text: a bare number for a plain price, the page's words with converted amounts otherwise. */
function cellText(cell: PriceCell, { language, rate }: Money): string {
  const prices = messagesFor(language).prices;
  if (cell.text === NOT_OFFERED) return "—";
  const pure = purePrice(cell.text);
  if (pure) return rate ? formatDongNumber(pure.usd * rate.usdToVnd, language) : formatPageDollars(pure.text, language);
  const text = rate ? priceText(cell.text, (usd) => formatDongPrice(usd * rate.usdToVnd, language)) : priceText(cell.text, null);
  return prices.term(text);
}

function Table({ table, money }: { table: PriceTable; money: Money }) {
  const prices = messagesFor(money.language).prices;
  const width = table.columns.length;
  return (
    <div className="uc-price-table-block">
      {table.caption ? <div className="uc-price-caption">{prices.term(table.caption)}</div> : null}
      <div className="uc-price-scroll">
        <table className="uc-price-table">
          <thead>
            <tr>
              {table.columns.map((column, index) => (
                <th key={index} scope="col" className={`is-${column.kind} is-${column.tone}`}>
                  {prices.term(column.label)}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {table.rows.map((row, index) =>
              row.kind === "heading" ? (
                <tr key={index} className="is-heading">
                  <td colSpan={width}>{prices.term(row.label)}</td>
                </tr>
              ) : (
                <tr key={index} className={row.shaded ? "is-shaded" : undefined}>
                  {row.labels.map((cell) => (
                    <th key={cell.column} scope="row" rowSpan={cell.rowSpan} className={cell.column === 0 ? "is-name" : "is-label"} {...tooltipProps(cell.description)}>
                      <span>{cell.column === 0 ? cell.text : prices.term(cell.text)}</span>
                      {cell.tag ? <span className="uc-price-tag">{prices.term(cell.tag)}</span> : null}
                    </th>
                  ))}
                  {row.prices.map((cell) => (
                    <td key={cell.column} className={`uc-num${cell.text === NOT_OFFERED ? " is-missing" : ""}${purePrice(cell.text) ? " is-number" : " is-text"}`}>
                      {cellText(cell, money)}
                    </td>
                  ))}
                </tr>
              ),
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}

export function PricesTab() {
  const settings = useSettings();
  const language = settings.language;
  const messages = messagesFor(language);
  const rate = useApp((state) => state.exchangeRate);
  const provider = settings.priceProvider;
  const tiers = priceTiers(provider);
  const tier = tiers.includes(settings.priceTier) ? settings.priceTier : tiers[0]!;
  const dongAvailable = showsDong(language, rate);
  const dong = dongAvailable && settings.priceCurrency === "vnd";
  const money = useMemo<Money>(() => ({ language, rate: dong ? rate : null }), [language, rate, dong]);
  const sections = useMemo(() => priceSections(provider, tier), [provider, tier]);
  const source = priceSource(provider);
  const host = new URL(source).host;
  const accent = PROVIDER_BRANDS[provider].color;

  return (
    <>
      <div className="uc-price-controls">
        <Capsules
          label={messages.prices.providerLabel}
          options={PRICE_PROVIDERS.map((value) => ({
            value,
            label: PROVIDER_BRANDS[value].name,
            icon: <ProviderMark brand={PROVIDER_BRANDS[value].brand} size={11} />,
            color: PROVIDER_BRANDS[value].color,
          }))}
          value={provider}
          onChange={(value) => updateSettings({ priceProvider: value })}
        />
        <div className="uc-price-row">
          <Capsules
            small
            label={messages.prices.tierLabel}
            options={tiers.map((value) => ({ value, label: messages.prices.tier(value), color: accent }))}
            value={tier}
            onChange={(value) => updateSettings({ priceTier: value })}
          />
          {dongAvailable ? (
            <Capsules
              small
              label={messages.prices.currencyLabel}
              options={(["vnd", "usd"] as const).map((value) => ({ value, label: messages.prices.currency(value), color: VIEW_COLORS.prices }))}
              value={settings.priceCurrency}
              onChange={(value) => updateSettings({ priceCurrency: value })}
            />
          ) : null}
        </div>
        <p className="uc-usage-note is-left">
          {messages.prices.unitNote(dong ? "vnd" : "usd")}
          {dong && rate ? ` ${rateLine(rate, language, settings.timeFormat)}` : ""}
        </p>
      </div>
      {sections.map((section) => (
        <section key={section.id} className="uc-section" style={{ "--uc-price-accent": accent } as CSSProperties}>
          <ChipHeader icon={<TagIcon size={11} />} color={accent} title={messages.prices.section(section.id)} />
          <div className="uc-card uc-price-card">
            {section.tables.map((table, index) => (
              <Table key={index} table={table} money={money} />
            ))}
          </div>
        </section>
      ))}
      <button type="button" className="uc-link-button is-block" onClick={() => void backend().openUrl(source)} aria-label={messages.prices.openSource(host)}>
        {messages.prices.source(host, dayText(PRICES_RETRIEVED_AT, language))}
        <ExternalIcon size={10} />
      </button>
    </>
  );
}
