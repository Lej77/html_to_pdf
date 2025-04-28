// See https://aka.ms/new-console-template for more information

try
{
    using Stream stdin = Console.OpenStandardInput();
    using Stream stdout = Console.OpenStandardOutput();
    iText.Html2pdf.HtmlConverter.ConvertToPdf(stdin, stdout);
}
catch (Exception ex)
{
    Console.Error.WriteLine(ex);
    Environment.Exit(1);
}
Environment.Exit(0);
