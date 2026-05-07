import type { Metadata } from 'next';
import './globals.css';

export const metadata: Metadata = {
  title: 'FlashPool — SOL/USD Prediction Market',
  description: 'High-performance on-chain prediction market powered by Pyth oracles and MagicBlock Ephemeral Rollups. Place predictions on SOL/USD price movements.',
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <head>
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <meta name="theme-color" content="#080b12" />
        <link rel="preconnect" href="https://fonts.googleapis.com" />
        <link rel="preconnect" href="https://fonts.gstatic.com" crossOrigin="anonymous" />
      </head>
      <body>{children}</body>
    </html>
  );
}
