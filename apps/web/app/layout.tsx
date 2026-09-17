import type { Metadata } from "next";
import type { ReactNode } from "react";

import { DEFAULT_LOCALE, LOCALE_BOOTSTRAP_SCRIPT } from "../copy/locales";
import { SERVER_THEME_BOOTSTRAP_SCRIPT } from "../platform/server-theme-cache";
import "../styles/product.css";
import "../styles/product-v2.css";
import "../styles/v3/index.css";

export const metadata: Metadata = {
  title: "rove",
  description: "Local-first agent product shell for workspaces, sessions, and runs.",
};

/** Swallow chrome-extension:// script errors so the Next dev overlay stays usable. */
const SUPPRESS_EXTENSION_ERRORS_SCRIPT = `try{var isExt=function(s){return typeof s==="string"&&s.indexOf("chrome-extension://")===0};window.addEventListener("error",function(e){if(isExt(e.filename)||isExt(e.message)){e.preventDefault();e.stopImmediatePropagation()}},true);window.addEventListener("unhandledrejection",function(e){var r=e.reason;var m=r&&(r.stack||r.message||String(r));if(isExt(m)){e.preventDefault();e.stopImmediatePropagation()}},true)}catch{}`;

export default function RootLayout({
  children,
}: Readonly<{
  children: ReactNode;
}>) {
  return (
    <html lang={DEFAULT_LOCALE} data-theme="light" suppressHydrationWarning>
      <head>
        <script
          dangerouslySetInnerHTML={{
            __html: `${SUPPRESS_EXTENSION_ERRORS_SCRIPT}${SERVER_THEME_BOOTSTRAP_SCRIPT}${LOCALE_BOOTSTRAP_SCRIPT}`,
          }}
        />
      </head>
      <body>{children}</body>
    </html>
  );
}
