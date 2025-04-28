# HtmlToPdf_Framework

This C# .Net console program reads a HTML file from stdin and converts it to a PDF that it writes to stdout.

## Build

It is possible to use the `dotnet` CLI to build (but not publish an installer for) this program.

```bash
dotnet restore --packages ./packages
dotnet build --configuration Release
```
