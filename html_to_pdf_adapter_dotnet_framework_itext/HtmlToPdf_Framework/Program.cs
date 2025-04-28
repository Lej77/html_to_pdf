using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using System.Threading.Tasks;

using System.IO;
using iTextSharp.text;
using System.Xml.Linq;

namespace HtmlToPdf_Framework
{
    internal class Program
    {
        static void Main(string[] args)
        {
            if (args.Length > 0 && args[0].ToLower() == "help")
            {
                Console.WriteLine(".Net Framework program that reads HTML from stdin and writes PDFs to stdout.");
                Console.WriteLine("");
                Console.WriteLine("First argument specifes the mode, it can be one of:");
                Console.WriteLine($"{PDFWriteMode.Default} (alias {(int)PDFWriteMode.Default}): use the default mode, currently {DEFAULT_PDF_WRITE_MODE} but might change");
                Console.WriteLine($"{PDFWriteMode.HTMLParse_ObsoleteHTMLParser} (alias {(int)PDFWriteMode.HTMLParse_ObsoleteHTMLParser}): uses iTextSharp.text.html.simpleparser.HTMLWorker");
                Console.WriteLine($"{PDFWriteMode.HTMLParse_XMLWorkerSimple} (alias {(int)PDFWriteMode.HTMLParse_XMLWorkerSimple}): uses iTextSharp.tool.xml.XMLWorkerHelper");
                Console.WriteLine($"{PDFWriteMode.HTMLParse_XMLWorkerAdvanced} (alias {(int)PDFWriteMode.HTMLParse_XMLWorkerAdvanced}): uses iTextSharp.tool.xml.XMLWorkerHelper with empty CSS");
                Console.WriteLine("");
                Console.WriteLine("Second argument is an optional separator for the input text that will be used to split it into multiple pages");
                return;
            }
            try
            {
                var mode = PDFWriteMode.Default;
                if (args.Length > 0)
                {
                    var arg = args[0];
                    if (int.TryParse(arg, out int number))
                    {
                        if (Enum.IsDefined(typeof(PDFWriteMode), number))
                        {
                            mode = (PDFWriteMode)number;
                        }
                        else
                        {
                            Console.Error.WriteLine($"1st argument is invalid, found \"{args[0]}\" but expected a number in the range 0 to {Enum.GetNames(typeof(PDFWriteMode)).Length - 1} where 0 is an alias for the default output");
                            Environment.Exit(3);
                        }
                    }
                    else if (Enum.TryParse(arg, out PDFWriteMode value))
                    {
                        mode = value;
                    }
                    else
                    {
                        Console.Error.WriteLine($"1st argument is invalid, found \"{args[0]}\" but expected a number or the name of a format");
                        Environment.Exit(3);
                    }
                }
                string separator = null;
                if (args.Length > 1)
                {
                    separator = args[1];
                }
                string[] inData;
                using (var stdin = Console.OpenStandardInput())
                {
                    using (var textReader = new StreamReader(stdin, Encoding.UTF8, true))
                    {
                        var text = textReader.ReadToEnd();
                        if (string.IsNullOrEmpty(separator))
                        {
                            inData = new string[] { text };
                        }
                        else
                        {
                            inData = text.Split(new[] { separator }, StringSplitOptions.None);
                        }
                    }
                }
                using (var stdout = Console.OpenStandardOutput())
                {
                    getPDFData(inData, stdout, mode);
                    // Can use temporary buffer instead of stdout as output stream to ignore System.NotSupportedException: Stream does not support writing.
                    // This likely because we are closing the stream early or something (memory stream probably ignores close and so works anyway).
                    // When this happens we only get part of the HTML convert to a PDF, so its likely that the output is closed early when there are errors.
                    //
                    // var data = getPDFData(inData, mode);
                    // stdout.Write(data, 0, data.Length);
                }
            }
            catch (Exception ex)
            {
                Console.Error.WriteLine(ex);
                if (ex is PDFWriterException)
                {
                    Environment.Exit(2);
                }
                else
                {
                    Environment.Exit(1);
                }
            }
            Environment.Exit(0);
        }

        public class PDFWriterException : Exception
        {
            public PDFWriterException(string message, Exception innerException) : base(message, innerException)
            { }
        }

        public enum PDFWriteMode
        {
            Default = 0,
            HTMLParse_ObsoleteHTMLParser,
            HTMLParse_XMLWorkerSimple,
            HTMLParse_XMLWorkerAdvanced,
        }
        public readonly static PDFWriteMode DEFAULT_PDF_WRITE_MODE = PDFWriteMode.HTMLParse_XMLWorkerSimple;

        /// <summary>
        /// This code converts HTML text to a PDF file.
        /// It uses the library "iTextSharp" (for pdf work) and "iTextSharp.xmlworker" (for HTML parsing) from NuGet.
        /// Code was copied from the internet:
        /// http://stackoverflow.com/questions/25164257/how-to-convert-html-to-pdf-using-itextsharp
        /// </summary>
        /// <param name="HTMLText">HTML text to parse.</param>
        /// <param name="HTMLParseMethod">Determines which method is used to parse the HTML data. (Options: 0, 1, any other)</param>
        /// <param name="ErrorHandler">Handle any caught errors.</param>
        /// <returns>PDF document data.</returns>
        private static byte[] getPDFData(string HTMLText, PDFWriteMode HTMLParseMethod = PDFWriteMode.Default, Action<Exception> ErrorHandler = null)
        {
            return getPDFData(new string[] { HTMLText }, HTMLParseMethod, ErrorHandler);
        }
        /// <summary>
        /// This code converts HTML text to a PDF file.
        /// It uses the library "iTextSharp" (for pdf work) and "iTextSharp.xmlworker" (for HTML parsing) from NuGet.
        /// Code was copied from the internet:
        /// http://stackoverflow.com/questions/25164257/how-to-convert-html-to-pdf-using-itextsharp
        /// </summary>
        /// <param name="HTMLTexts">HTML text to parse.</param>
        /// <param name="HTMLParseMethod">Determines which method is used to parse the HTML data. (Options: 0, 1, any other)</param>
        /// <param name="ErrorHandler">Handle any caught errors.</param>
        /// <returns>PDF document data.</returns>
        private static byte[] getPDFData(string[] HTMLTexts, PDFWriteMode HTMLParseMethod = PDFWriteMode.Default, Action<Exception> ErrorHandler = null)
        {
            //Create a byte array that will eventually hold our final PDF
            Byte[] bytes;

            //Boilerplate iTextSharp setup here
            //Create a stream that we can write to, in this case a MemoryStream
            using (var ms = new MemoryStream())
            {
                try
                {
                    getPDFData(HTMLTexts, ms, HTMLParseMethod);
                }
                catch (PDFWriterException ex)
                {
                    ErrorHandler?.Invoke(ex);
                }

                //After all of the PDF "stuff" above is done and closed but **before** we
                //close the MemoryStream, grab all of the active bytes from the stream
                bytes = ms.ToArray();
            }

            return bytes;
        }

        private static void getPDFData(string[] HTMLTexts, Stream output, PDFWriteMode HTMLParseMethod = PDFWriteMode.Default)
        {
            if (HTMLParseMethod != PDFWriteMode.HTMLParse_ObsoleteHTMLParser &&
                HTMLParseMethod != PDFWriteMode.HTMLParse_XMLWorkerAdvanced &&
                HTMLParseMethod != PDFWriteMode.HTMLParse_XMLWorkerSimple)
            {
                // Unknown value, fallback to default:
                HTMLParseMethod = PDFWriteMode.Default;
            }

            if (HTMLParseMethod == PDFWriteMode.Default)
            {
                // Default parse method:
                HTMLParseMethod = DEFAULT_PDF_WRITE_MODE;
            }


            // HTMLParse_ObsoleteHTMLParser (example 1) doesn't support "<hr>" tags. It will throw an exception of one is used.
            if (HTMLParseMethod == PDFWriteMode.HTMLParse_ObsoleteHTMLParser)
            {
                for (int iii = 0; iii < HTMLTexts.Length; iii++)
                {
                    HTMLTexts[iii] = HTMLTexts[iii].Replace("<hr />", "").Replace("<hr>", "").Replace("</hr>", "");
                }
            }

            try
            {
                //Create an iTextSharp Document which is an abstraction of a PDF but **NOT** a PDF
                using (var doc = new Document())
                {
                    //Create a writer that's bound to our PDF abstraction and our stream
                    using (var writer = iTextSharp.text.pdf.PdfWriter.GetInstance(doc, output))
                    {
                        //Open the document for writing
                        doc.Open();

                        foreach (string HTMLText in HTMLTexts)
                        {
                            // [Edit]: Requested new page
                            doc.NewPage();

                            /*
                            //Our sample HTML and CSS
                            var example_html = @"<p>This <em>is </em><span class=""headline"" style=""text-decoration: underline;"">some</span> <strong>sample <em> text</em></strong><span style=""color: red;"">!!!</span></p>";
                            var example_css = @".headline{font-size:200%}";
                            */

                            if (HTMLParseMethod == PDFWriteMode.HTMLParse_ObsoleteHTMLParser)
                            {


                                /**************************************************
                                    * Example #1                                     *
                                    *                                                *
                                    * Use the built-in HTMLWorker to parse the HTML. *
                                    * Only inline CSS is supported.                  *
                                    * ************************************************/
                                /*
                                //Create a new HTMLWorker bound to our document
                                using (var htmlWorker = new iTextSharp.text.html.simpleparser.HTMLWorker(doc))
                                {

                                    //HTMLWorker doesn't read a string directly but instead needs a TextReader (which StringReader subclasses)
                                    using (var sr = new StringReader(example_html))
                                    {

                                        //Parse the HTML
                                        htmlWorker.Parse(sr);
                                    }
                                }
                                */

                                //Create a new HTMLWorker bound to our document
                                using (var htmlWorker = new iTextSharp.text.html.simpleparser.HTMLWorker(doc))
                                {

                                    //HTMLWorker doesn't read a string directly but instead needs a TextReader (which StringReader subclasses)
                                    using (var sr = new StringReader(HTMLText))
                                    {

                                        //Parse the HTML
                                        htmlWorker.Parse(sr);
                                    }
                                }



                            }
                            else if (HTMLParseMethod == PDFWriteMode.HTMLParse_XMLWorkerSimple)
                            {

                                /**************************************************
                                    * Example #2                                     *
                                    *                                                *
                                    * Use the XMLWorker to parse the HTML.           *
                                    * Only inline CSS and absolutely linked          *
                                    * CSS is supported                               *
                                    * ************************************************/


                                //XMLWorker also reads from a TextReader and not directly from a string
                                using (var srHtml = new StringReader(HTMLText))
                                {

                                    //Parse the HTML
                                    iTextSharp.tool.xml.XMLWorkerHelper.GetInstance().ParseXHtml(writer, doc, srHtml);
                                }
                            }
                            else if (HTMLParseMethod == PDFWriteMode.HTMLParse_XMLWorkerAdvanced)
                            {
                                /**************************************************
                                    * Example #3                                     *
                                    *                                                *
                                    * Use the XMLWorker to parse HTML and CSS        *
                                    * ************************************************/
                                /*
                                //In order to read CSS as a string we need to switch to a different constructor
                                //that takes Streams instead of TextReaders.
                                //Below we convert the strings into UTF8 byte array and wrap those in MemoryStreams
                                using (var msCss = new MemoryStream(System.Text.Encoding.UTF8.GetBytes(example_css)))
                                {
                                    using (var msHtml = new MemoryStream(System.Text.Encoding.UTF8.GetBytes(example_html)))
                                    {

                                        //Parse the HTML
                                        iTextSharp.tool.xml.XMLWorkerHelper.GetInstance().ParseXHtml(writer, doc, msHtml, msCss);
                                    }
                                }
                                */

                                using (var msCss = new MemoryStream(System.Text.Encoding.UTF8.GetBytes("")))
                                {
                                    using (var msHtml = new MemoryStream(System.Text.Encoding.UTF8.GetBytes(HTMLText)))
                                    {
                                        //Parse the HTML
                                        iTextSharp.tool.xml.XMLWorkerHelper.GetInstance().ParseXHtml(writer, doc, msHtml, msCss);
                                    }
                                }
                            }
                        }

                        if (!output.CanWrite || !doc.IsOpen())
                        {
                            throw new Exception("PDF document was closed while still being written, it is likely that this was caused by the HTML parser encountered a bug");
                        }

                        writer.Flush();
                        doc.Close();
                    }
                }
            }
            catch (Exception ex)
            {
                throw new PDFWriterException("Failed to convert HTML to PDF.", ex);
            }
        }
    }
}
