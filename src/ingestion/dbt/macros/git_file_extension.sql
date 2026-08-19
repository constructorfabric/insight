{# The extension of a git path: the segment after the final '.' of the file
   NAME, empty when the name carries none.

   Splitting the whole path would read a dot in a directory (`github.com/x/y`,
   `v1.2/setup`) as the extension separator and return the rest of the path, so
   the basename is taken first. Dotfiles (`.gitignore`) and extensionless names
   (`Makefile`) have no extension and yield ''. #}

{% macro git_file_extension(path_expr) -%}
{%- set basename = "arrayElement(splitByChar('/', " ~ path_expr ~ "), -1)" -%}
if(
        length(splitByChar('.', {{ basename }})) > 1
            AND arrayElement(splitByChar('.', {{ basename }}), 1) != '',
        arrayElement(splitByChar('.', {{ basename }}), -1),
        ''
    )
{%- endmacro %}
