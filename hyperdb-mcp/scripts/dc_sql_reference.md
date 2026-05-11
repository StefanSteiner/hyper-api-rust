# Salesforce Data Cloud SQL Reference

_Scraped from: https://developer.salesforce.com/docs/data/data-cloud-query-guide/references/dc-sql-reference/data-cloud-sql-context.html_

_To refresh: `uv run --with playwright python3 scripts/scrape_dc_sql_reference.py`_



---

# Data 360 SQL and Tableau Hyper API | Data 360 SQL Reference | Data 360 Query Guide

_URL: https://developer.salesforce.com/docs/data/data-cloud-query-guide/references/dc-sql-reference/data-cloud-sql-context.html_

## Data 360 SQL Reference

As of October 14, 2025, Data Cloud has been rebranded to Data 360. During this transition, you may see references to Data Cloud in our application and documentation. While the name is new, the functionality and content remains unchanged.

Note

Data 360 SQL supports the Data 360 Query API and the Tableau Hyper API. This reference guide describes Data 360 SQL and how it applies to both APIs.

Use Data 360 SQL to query data in Data 360, answer analytical questions, or derive insights from your data.

Data 360 SQL works with these resources.

- Integrated apps such as Query Editor, DBeaver, or Tableau

- Custom apps built with language-specific libraries on Data 360

For more information, see How Do I Query Data in Data 360? and Data 360 SQL Query APIs.

Use Tableau Hyper API to access data in Tableau for analytics and data transformations.

Tableau Hyper API uses Data 360 SQL to complete these tasks.

- Make data in unsupported data sources accessible to Tableau.

- Automate custom extract, transform, and load (ETL) processes.

- Implement rolling window updates or custom incremental updates.

For more information, see the Tableau Hyper API documentation.



---

# Data 360 SQL Syntax | Data 360 SQL Reference | Data 360 Query Guide

_URL: https://developer.salesforce.com/docs/data/data-cloud-query-guide/references/dc-sql-reference/sql-commands-context.html_

## Data 360 SQL Syntax

This section describes the standard syntax for creating Data 360 SQL statements and the syntax for the read-only statements available in Data 360 and Tableau Hyper API.

- Syntax

- SELECT

- VALUES



---

# Data Types | Data 360 SQL Reference | Data 360 Query Guide

_URL: https://developer.salesforce.com/docs/data/data-cloud-query-guide/references/dc-sql-reference/datatypes.html_

## Data Types

The Data 360 SQL type system includes two groups of types.

- Atomic types that describe single values

- Composite types that describe collections of values

Atomic types are basic data types. This table lists all available atomic types.

Name

Aliases

Description

BIGINT

```
BIGINT
```

INT8

```
INT8
```

Signed eight-byte integer

BOOLEAN

```
BOOLEAN
```

BOOL

```
BOOL
```

Boolean value with ternary logic (true/false/unknown)

BYTES

```
BYTES
```

Binary data (“byte array”)

CHARACTER [ (n) ]

```
CHARACTER [ (n) ]
```

CHAR [ (n) ]

```
CHAR [ (n) ]
```

Fixed-length character string

CHARACTER VARYING (n)

```
CHARACTER VARYING (n)
```

VARCHAR (n)

```
VARCHAR (n)
```

Variable-length character string with limit

DATE

```
DATE
```

Calendar date (year, month, day)

REAL

```
REAL
```

FLOAT4

```
FLOAT4
```

Single-precision floating-point number (4 bytes)

DOUBLE PRECISION

```
DOUBLE PRECISION
```

FLOAT, FLOAT8

```
FLOAT
```

```
FLOAT8
```

Double-precision floating-point number (8 bytes)

INTEGER

```
INTEGER
```

INT, INT4

```
INT
```

```
INT4
```

Signed four-byte integer

INTERVAL

```
INTERVAL
```

Time span, not supported in Tableau

NUMERIC [ (p, s) ]

```
NUMERIC [ (p, s) ]
```

DECIMAL [ (p, s) ]

```
DECIMAL [ (p, s) ]
```

Exact numeric of selectable precision

SMALLINT

```
SMALLINT
```

INT2

```
INT2
```

Signed two-byte integer

TEXT

```
TEXT
```

Variable-length character string

TIME [ WITHOUT TIME ZONE ]

```
TIME [ WITHOUT TIME ZONE ]
```

Time of day (no time zone)

TIMESTAMP [ WITHOUT TIME ZONE ]

```
TIMESTAMP [ WITHOUT TIME ZONE ]
```

Date and time (no time zone)

TIMESTAMP WITH TIME ZONE

```
TIMESTAMP WITH TIME ZONE
```

TIMESTAMPTZ

```
TIMESTAMPTZ
```

Date and time, including time zone

Persisting NUMERIC values into .hyper files with a precision greater than 18 by using Tableau Hyper API requires at least database version 3.

```
NUMERIC
```

```
.hyper
```

Persisting 32-bit floating point values into .hyper files by using Tableau Hyper API requires at least database version 4.

```
.hyper
```

Note

Composite types are collections of multiple data items in a single SQL value. You can use them for dedicated schema denormalization, which is useful for specific domains, such as machine learning applications. Data 360 SQL supports the array composite type.

An array is an ordered sequence of values. Arrays in Data 360 SQL are strongly typed and can be built from all supported atomic types. For more details, see Array Type.



---

# Scalar Functions | Data 360 SQL Reference | Data 360 Query Guide

_URL: https://developer.salesforce.com/docs/data/data-cloud-query-guide/references/dc-sql-reference/scalar-functions.html_

## Scalar Functions and Operators

This section describes functions and operators for the built-in data types.



---

# Aggregate Functions | Data 360 SQL Reference | Data 360 Query Guide

_URL: https://developer.salesforce.com/docs/data/data-cloud-query-guide/references/dc-sql-reference/aggregate.html_

## Aggregate Functions

Applies to: ✅ Data 360 SQL ✅ Tableau Hyper API

Functions that compute a single result from a set of input values. Use aggregate functions to:

- Summarize data by computing counts, sums, and averages.

- Analyze distributions with statistical functions like standard deviation and variance.

- Find extremes with MIN and MAX values.

- Compute percentiles and identify modes in datasets.

- NULL handling: Except for COUNT, aggregate functions return NULL if no rows are selected. Use COALESCE to substitute a default value.

```
COUNT
```

```
COALESCE
```

- GROUP BY requirement: Non-aggregated fields in the SELECT clause must appear in the GROUP BY clause.

```
SELECT
```

```
GROUP BY
```

- FILTER clause: Use the FILTER clause to apply conditions to individual aggregates.

Common aggregate functions for summarizing data:

- ANY_VALUE - Return an arbitrary value from the group.

- APPROX_COUNT_DISTINCT - Approximate count of distinct values.

- AVG - Calculate the arithmetic mean.

- BIT_AND - Compute bitwise AND of all values.

- BIT_OR - Compute bitwise OR of all values.

- BOOL_AND - True if all values are true.

- BOOL_OR - True if any value is true.

- COUNT - Count rows or non-null values.

- EVERY - True if all values are true (alias for BOOL_AND).

- MAX - Find the maximum value.

- MIN - Find the minimum value.

- SUM - Calculate the total.

Functions for statistical analysis:

- CORR - Compute correlation coefficient.

- COVAR_POP - Compute population covariance.

- COVAR_SAMP - Compute sample covariance.

- STDDEV/STDDEV_SAMP - Compute sample standard deviation.

- STDDEV_POP - Compute population standard deviation.

- VAR_POP - Compute population variance.

- VARIANCE/VAR_SAMP - Compute sample variance.

Cast the input of an aggregate function to force a different output type. For example, use VAR_POP(CAST(A AS DOUBLE PRECISION)) to get a double precision result.

```
VAR_POP(CAST(A AS DOUBLE PRECISION))
```

```
double precision
```

Tip

Aggregate functions that use the WITHIN GROUP syntax:

```
WITHIN GROUP
```

- MODE - Return the most frequent value.

- PERCENTILE_CONT - Compute continuous percentile with interpolation.

- PERCENTILE_DISC - Compute discrete percentile without interpolation.

All ordered-set aggregates ignore NULL values in their sorted input. The fraction parameter must be between 0 and 1.

- GROUPING - Identify which columns are aggregated in grouping sets.

- FILTER Clause - Filter rows passed to aggregate functions.

Calculate summary statistics.

```
1SELECT
2    count(*) AS total_orders,
3    sum(amount) AS total_revenue,
4    avg(amount) AS avg_order_value,
5    min(amount) AS min_order,
6    max(amount) AS max_order
7FROM orders;
```

Summarize data by category.

```
1SELECT
2    category,
3    count(*) AS product_count,
4    avg(price) AS avg_price
5FROM products
6GROUP BY category;
```

Compute multiple filtered aggregates in one query.

```
1SELECT
2    count(*) AS total,
3    count(*) FILTER (WHERE status = 'completed') AS completed,
4    count(*) FILTER (WHERE status = 'pending') AS pending
5FROM orders;
```

Compute standard deviation and variance.

```
1SELECT
2    stddev(score) AS score_stddev,
3    variance(score) AS score_variance
4FROM test_results;
```

Find median and quartiles.

```
1SELECT
2    percentile_cont(0.25) WITHIN GROUP (ORDER BY salary) AS q1,
3    percentile_cont(0.5) WITHIN GROUP (ORDER BY salary) AS median,
4    percentile_cont(0.75) WITHIN GROUP (ORDER BY salary) AS q3
5FROM employees;
```

- GROUP BY Clause

- Window Functions



---

# Window Functions | Data 360 SQL Reference | Data 360 Query Guide

_URL: https://developer.salesforce.com/docs/data/data-cloud-query-guide/references/dc-sql-reference/window.html_

## Window Functions and Queries

Applies to: ✅ Data 360 SQL ✅ Tableau Hyper API

Window functions provide the ability to perform calculations across sets of rows that are related to the current query row. This section introduces window queries and a reference for the window functions supported in Data 360 SQL.

A window function does a calculation across a set of table rows that are related to the current row. Rows in window function calculation remain distinct in the output.

Here's an example that shows how to compare each employee's salary with the average salary in their department:

```
1SELECT depname, empno, salary, avg(salary) OVER (PARTITION BY depname) FROM empsalary;
```

Result:

```
1depname   | empno | salary |          avg
2----------+-------+--------+-----------------------
3develop   |    11 |   5200 | 5020.0000000000000000
4develop   |     7 |   4200 | 5020.0000000000000000
5develop   |     9 |   4500 | 5020.0000000000000000
6develop   |     8 |   6000 | 5020.0000000000000000
7develop   |    10 |   5200 | 5020.0000000000000000
8personnel |     5 |   3500 | 3700.0000000000000000
9personnel |     2 |   3900 | 3700.0000000000000000
10sales     |     3 |   4800 | 4866.6666666666666667
11sales     |     1 |   5000 | 4866.6666666666666667
12sales     |     4 |   4800 | 4866.6666666666666667
13(10 rows)
```

The first three output columns come directly from the table empsalary, and there's one output row for each row in the table. The fourth column represents an average taken across all the table rows that have the same depname value as the current row.

```
empsalary
```

```
depname
```

A window function call always contains an OVER clause directly after the window function's name and arguments. The OVER clause defines how the rows of the query are split up for processing by the window function. The PARTITION BY clause within OVER divides the rows into groups that share the same values of the PARTITION BY expressions. For each row, the window function is computed across ‌rows that fall into the same partition as the current row.

```
OVER
```

```
PARTITION BY
```

```
OVER
```

```
PARTITION BY
```

You can also control the order in which rows are processed by window functions using ORDER BY within OVER. Here's an example:

```
ORDER BY
```

```
OVER
```

```
1SELECT depname, empno, salary, rank() OVER (PARTITION BY depname ORDER BY salary DESC)
2FROM empsalary;
```

Result:

```
1depname   | empno | salary | rank
2----------+-------+--------+-----
3develop   |     8 |   6000 |    1
4develop   |    10 |   5200 |    2
5develop   |    11 |   5200 |    2
6develop   |     9 |   4500 |    4
7develop   |     7 |   4200 |    5
8personnel |     2 |   3900 |    1
9personnel |     5 |   3500 |    2
10sales     |     1 |   5000 |    1
11sales     |     4 |   4800 |    2
12sales     |     3 |   4800 |    2
13(10 rows)
```

In this example, the rank function produces a numerical rank for each unique ORDER BY value in the current row's partition, using the order defined by the ORDER BY clause.The OVER clause defines the behavior of rank.

```
rank
```

```
ORDER BY
```

```
OVER
```

```
rank
```

You can specify ORDER BY independently top-level query ORDER BY output.

```
ORDER BY
```

The query's FROM clause filtered by its WHERE, GROUP BY, and HAVING clauses determine the rows computed in the window function. For example, a row removed because it doesn't meet the WHERE condition isn't seen by any window functions. A query can have many window functions that divide the data in different ways using different OVER clauses. But they all work on the same group of rows defined by the virtual table.

```
FROM
```

```
WHERE
```

```
GROUP BY
```

```
HAVING
```

```
WHERE
```

It's also possible to omit PARTITION BY, in which case there's a single partition containing all rows.

```
PARTITION BY
```

A window frame is a set of rows within the defined partition. Some window functions act only on the rows of the window frame, rather than on the whole partition. If ORDER BY is given, the frame consists of all rows from the start of the partition up to the current row. It also includes any rows that are equal to the current row. If you don’t include ORDER BY, the default frame includes of all rows in the partition. Here's an example using sum:

```
ORDER BY
```

```
sum
```

```
1SELECT salary, sum(salary) OVER () FROM empsalary;
```

Result:

```
1salary |  sum
2-------+-------
3  5200 | 47100
4  5000 | 47100
5  3500 | 47100
6  4800 | 47100
7  3900 | 47100
8  4200 | 47100
9  4500 | 47100
10  4800 | 47100
11  6000 | 47100
12  5200 | 47100
13(10 rows)
```

The SELECT statement includes all rows in the table because there is no ORDER BY or PARTITION BY clauses. Each sum is taken over the whole table, and so we get the same result for each output row. But if we add an ORDER BY clause, we get different results:

```
SELECT
```

```
ORDER BY or PARTITION BY
```

```
ORDER BY
```

```
1SELECT salary, sum(salary) OVER (ORDER BY salary) FROM empsalary;
```

Result:

```
1salary |  sum
2-------+-------
3  3500 |  3500
4  3900 |  7400
5  4200 | 11600
6  4500 | 16100
7  4800 | 25700
8  4800 | 25700
9  5000 | 30700
10  5200 | 41100
11  5200 | 41100
12  6000 | 47100
13(10 rows)
```

Here the sum is taken from the first (lowest) salary up through the current one, including any duplicates of the current one.

You can only include window functions in the SELECT list and the ORDER BY clause of the query. Also, window functions execute after non-window aggregate functions. This means you can include an aggregate function call in the arguments of a window function. But you can’t include a window function in an aggregate function call.

```
SELECT
```

```
ORDER BY
```

To filter or group rows after the window calculations are performed, you can use a sub-select. For example:

```
1SELECT depname, empno, salary, enroll_date
2FROM
3   (SELECT depname, empno, salary, enroll_date,
4   rank() OVER (PARTITION BY depname ORDER BY salary DESC, empno) AS pos
5   FROM empsalary
6   ) AS ss
7WHERE pos < 3;
```

The preceding query only shows the rows from the inner query having rank of less than three.

```
rank
```

If a query has many window functions, name each windowing behavior in a WINDOW clause and then reference it in OVER. For example:

```
WINDOW
```

```
OVER
```

```
1SELECT sum(salary) OVER w, avg(salary) OVER w
2FROM empsalary
3WINDOW w AS (PARTITION BY depname ORDER BY salary DESC);
```

For more details about WINDOW clauses, see SELECT.

```
WINDOW
```

The built-in window functions are listed in the next table. Invoke these functions using window function syntax.

Function

Return Type

Description

row_number()

```
row_number()
```

bigint

```
bigint
```

number of the current row within its partition, counting from 1

rank()

```
rank()
```

bigint

```
bigint
```

rank of the current row with gaps; same as row_number of its first peer

```
row_number
```

modified_rank()

```
modified_rank()
```

bigint

```
bigint
```

rank of the current row with gaps, but taking the lowest rank in case of ties; same as row_number of its last peer

```
row_number
```

dense_rank()

```
dense_rank()
```

bigint

```
bigint
```

rank of the current row without gaps; this function counts peer groups

percent_rank()

```
percent_rank()
```

double precision

```
double precision
```

relative rank of the current row: (rank - 1) / (total partition rows - 1)

```
rank
```

cume_dist()

```
cume_dist()
```

double precision

```
double precision
```

cumulative distribution: (number of partition rows preceding or peer with current row) / total partition rows

ntile(num_buckets integer)

```
ntile(num_buckets integer)
```

integer

```
integer
```

integer ranging from 1 to the argument value, dividing the partition as equally as possible

lag(value anyelement [, offset integer [, default anyelement ]])

```
lag(value anyelement [, offset integer [, default anyelement ]])
```

same type as value

```
same type as value
```

returns <value> evaluated at the row that is <offset> rows before the current row within the partition; if there is no such row, instead return <default> (which must be of the same type as <value>). Both <offset> and <default> are evaluated with respect to the current row. If omitted, <offset> defaults to 1 and <default> to null

```
<value>
```

```
<offset>
```

```
<default>
```

```
<value>
```

```
<offset>
```

```
<default>
```

```
<offset>
```

```
<default>
```

lead(value anyelement [, offset integer [, default anyelement ]])

```
lead(value anyelement [, offset integer [, default anyelement ]])
```

same type as value

```
same type as value
```

returns <value> evaluated at the row that is <offset> rows after the current row within the partition; if there is no such row, instead return <default> (which must be of the same type as <value>). Both <offset> and <default> are evaluated with respect to the current row. If omitted, <offset> defaults to 1 and <default> to null

```
<value>
```

```
<offset>
```

```
<default>
```

```
<value>
```

```
<offset>
```

```
<default>
```

```
<offset>
```

```
<default>
```

first_value(value any)

```
first_value(value any)
```

same type as value

```
same type as value
```

returns <value> evaluated at the row that is the first row of the window frame

```
<value>
```

last_value(value any)

```
last_value(value any)
```

same type as value

```
same type as value
```

returns <value> evaluated at the row that is the last row of the window frame

```
<value>
```

nth_value(value any, nth integer)

```
nth_value(value any, nth integer)
```

same type as value

```
same type as value
```

returns <value> evaluated at the row that is the <nth> row of the window frame (counting from either the first or the last row in the frame, depending on the FROM option); null if no such row

```
<value>
```

```
<nth>
```

```
FROM
```

You can also use most aggregate functions in a window function. For a list of built-in aggregate functions, see Aggregate Functions. Aggregate functions act as window functions if an OVER clause follows the function call.

```
OVER
```

All of the functions depend on the sort ordering specified by the ORDER BY clause of the window definition. Rows that aren't unique in the ORDER BY columns are peers. The four ranking functions (including cume_dist) are defined to give the same answer for all peer rows.

```
ORDER BY
```

```
cume_dist
```

The modified_rank function differs from rank in that it assigns the lowest rank of all entries in case of a tie. For example:

```
modified_rank
```

```
rank
```

```
1SELECT depname, empno, salary,
2   rank() OVER (PARTITION BY depname ORDER BY salary DESC),
3   modified_rank() OVER (PARTITION BY depname ORDER BY salary DESC)
4FROM empsalary;
```

Result:

```
1depname   | empno | salary | rank | modified_rank
2----------+-------+--------+------+--------------
3develop   |     8 |   6000 |    1 |             1
4develop   |    10 |   5200 |    2 |             3
5develop   |    11 |   5200 |    2 |             3
6develop   |     9 |   4500 |    4 |             4
7develop   |     7 |   4200 |    5 |             5
8personnel |     2 |   3900 |    1 |             1
9personnel |     5 |   3500 |    2 |             2
10sales     |     1 |   5000 |    1 |             1
11sales     |     4 |   4800 |    2 |             3
12sales     |     3 |   4800 |    2 |             3
13(10 rows)
```

The first_value, last_value, and nth_value consider only the rows within the window frame. By default, this contains the rows from the start of the partition through the last peer of the current row. You can redefine the frame by adding a suitable frame specification (RANGE, ROWS) to the OVER clause. For information about frame specifications, see Window Function Call Syntax.

```
first_value
```

```
last_value
```

```
nth_value
```

```
RANGE
```

```
ROWS
```

```
OVER
```

If you use an aggregate function as a window function, it aggregates over the rows within the current row's window frame. An aggregate used with ORDER BY and the default window frame definition produces a running sum type of behavior. To obtain aggregation over the whole partition, exclude ORDER BY or use ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING.

```
ORDER BY
```

```
ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING
```

cume_dist computes the fraction of partition rows that are less than or equal to the current row and its peers. percent_rank computes the fraction of partition rows that are less than the current row, assuming the current row doesn't exist in the partition.

```
cume_dist
```

```
percent_rank
```

Possible window function call syntax:

```
1<function_name>([ <expression> [, ...] ])
2    [ FROM { FIRST | LAST } ]
3    [ { RESPECT | IGNORE } NULLS ]
4    OVER { ( <window_name> ) | <window_definition> }
5<function_name>(*)
6    [ { RESPECT | IGNORE } NULLS ]
7    OVER { ( <window_name> ) | <window_definition> }
```

where <window_definition> has the syntax:

```
<window_definition>
```

```
1[ <window_name> ]
2[ PARTITION BY <expression> [, ...] ]
3[ ORDER BY <expression> [ ASC | DESC | USING <operator> ] [ NULLS { FIRST | LAST } ] [, ...] ]
4[ <frame_clause> ]
```

The optional <frame_clause> can be one of:

```
<frame_clause>
```

```
1{ RANGE | ROWS } <frame_start> [ <frame_exclusion> ]
2{ RANGE | ROWS } BETWEEN <frame_start> AND <frame_end> [ <frame_exclusion> ]
```

where <frame_start> and <frame_end> can be one of:

```
<frame_start>
```

```
<frame_end>
```

```
1UNBOUNDED PRECEDING
2<offset> PRECEDING
3CURRENT ROW
4<offset> FOLLOWING
5UNBOUNDED FOLLOWING
```

and <frame_exclusion> can be one of:

```
<frame_exclusion>
```

```
1EXCLUDE CURRENT ROW
2EXCLUDE GROUP
3EXCLUDE TIES
4EXCLUDE NO OTHERS
```

In the previous definitions, <expression> represents any value expression that doesn't contain window function calls.

```
<expression>
```

<window_name> is a reference to a named window specification defined in the query's WINDOW clause. Or you can give a full <window_definition> in parentheses. See SELECT. OVER wname isn't equivalent to OVER (wname ...). The latter copies and modifies the window definition, and is rejected if the referenced window specification includes a frame clause.

```
<window_name>
```

```
WINDOW
```

```
<window_definition>
```

```
OVER wname
```

```
OVER (wname ...)
```

The PARTITION BY clause groups the rows of the query into partitions, which are processed separately by the window function. Expressions in the PARTITION BY or ORDER BY clauses can't be output-column names or numbers. Without PARTITION BY, all rows produced by the query are treated as a single partition. The ORDER BY clause determines the order in which the rows of a partition are processed by the window function. Without ORDER BY, rows are processed in an unspecified order.

```
PARTITION BY
```

```
ORDER BY
```

```
PARTITION BY
```

```
ORDER BY
```

The <frame_clause> specifies the set of rows in the window frame, which is a subset of the current partition, for those window functions that act on the frame instead of the whole partition. The set of rows in the frame can vary depending on which row is the current row. The frame can be in RANGE, ROWS or GROUPS mode. The frame runs from the <frame_start> to the <frame_end>. If <frame_end> is omitted, the end defaults to CURRENT ROW.

```
<frame_clause>
```

```
RANGE
```

```
ROWS
```

```
GROUPS
```

```
<frame_start>
```

```
<frame_end>
```

```
CURRENT ROW
```

A <frame_start> of UNBOUNDED PRECEDING means that the frame starts with the first row of the partition, and a <frame_end> of UNBOUNDED FOLLOWING means that the frame ends with the last row of the partition.

```
<frame_start>
```

```
UNBOUNDED PRECEDING
```

```
<frame_end>
```

```
UNBOUNDED FOLLOWING
```

In RANGE or GROUPS mode, a <frame_start> of CURRENT ROW means the frame starts with the current row's first peer row (a row that the window's ORDER BY clause sorts as equivalent to the current row), while a <frame_end> of CURRENT ROW means the frame ends with the current row's last peer row. In ROWS mode, CURRENT ROW is the current row.

```
RANGE
```

```
GROUPS
```

```
<frame_start>
```

```
CURRENT ROW
```

```
ORDER BY
```

```
<frame_end>
```

```
CURRENT ROW
```

```
ROWS
```

```
CURRENT ROW
```

In the <offset> PRECEDING and <offset> FOLLOWING frame options, the <offset> is an expression without variables, aggregate functions, or window functions. The meaning of the <offset> depends on the frame mode:

```
<offset> PRECEDING
```

```
<offset> FOLLOWING
```

```
<offset>
```

- In ROWS mode, the <offset> returns a non-null, non-negative integer, and the option means that the frame starts or ends the specified number of rows before or after the current row.

In ROWS mode, the <offset> returns a non-null, non-negative integer, and the option means that the frame starts or ends the specified number of rows before or after the current row.

```
ROWS
```

```
<offset>
```

- In RANGE mode, the ORDER BY clause specifies exactly one column. The <offset> specifies the maximum difference between the value of that column in the current row and its value in preceding or following rows of the frame. The data type of the <offset> expression varies depending on the data type of the ordering column. For numeric ordering columns, it's the same type as the ordering column. For datetime ordering columns it is an interval. For example, if the ordering column is of type date or timestamp, write RANGE BETWEEN '1 day' PRECEDING AND '10 days' FOLLOWING. The <offset> is still required to be non-null and non-negative, but "non-negative" depends on its data type.

In RANGE mode, the ORDER BY clause specifies exactly one column. The <offset> specifies the maximum difference between the value of that column in the current row and its value in preceding or following rows of the frame. The data type of the <offset> expression varies depending on the data type of the ordering column. For numeric ordering columns, it's the same type as the ordering column. For datetime ordering columns it is an interval. For example, if the ordering column is of type date or timestamp, write RANGE BETWEEN '1 day' PRECEDING AND '10 days' FOLLOWING. The <offset> is still required to be non-null and non-negative, but "non-negative" depends on its data type.

```
RANGE
```

```
ORDER BY
```

```
<offset>
```

```
interval
```

```
date
```

```
timestamp
```

```
RANGE BETWEEN '1 day' PRECEDING AND '10 days' FOLLOWING
```

```
<offset>
```

In any case, the distance to the end of the frame is limited by the distance to the end of the partition. Rows near the end of the partition can contain fewer rows.

Notice that in ROWS mode, 0 PRECEDING and 0 FOLLOWING is equivalent to CURRENT ROW. This normally holds in RANGE mode as well, for an appropriate data-type-specific meaning of zero.

```
ROWS
```

```
0 PRECEDING
```

```
0 FOLLOWING
```

```
CURRENT ROW
```

```
RANGE
```

The <frame_exclusion> option allows rows around the current row to be excluded from the frame. EXCLUDE CURRENT ROW excludes the current row from the frame. EXCLUDE GROUP excludes the current row and its ordering peers from the frame. EXCLUDE TIES excludes any peers of the current row from the frame, but not the current row itself. EXCLUDE NO OTHERS defines the default behavior of not excluding the current row or its peers.

```
<frame_exclusion>
```

```
EXCLUDE CURRENT ROW
```

```
EXCLUDE GROUP
```

```
EXCLUDE TIES
```

```
EXCLUDE NO OTHERS
```

The default framing option is RANGE UNBOUNDED PRECEDING, which is the same as RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW. With ORDER BY, this sets the frame to be all rows from the partition start through the current row's last ORDER BY peer. Without ORDER BY, this means all rows of the partition are included in the window frame.

```
RANGE UNBOUNDED PRECEDING
```

```
RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
```

```
ORDER BY
```

<frame_start> can’t be UNBOUNDED FOLLOWING, <frame_end> can’t be UNBOUNDED PRECEDING, and the <frame_end> can’t be earlier in the list of <frame_start> and <frame_end> options than the <frame_start> is. For example RANGE BETWEEN CURRENT ROW AND offset PRECEDING isn’t allowed. But, ROWS BETWEEN 7 PRECEDING AND 8 PRECEDING is allowed, even though it never selects any rows.

```
<frame_start>
```

```
UNBOUNDED FOLLOWING
```

```
<frame_end>
```

```
UNBOUNDED PRECEDING
```

```
<frame_end>
```

```
<frame_start>
```

```
<frame_end>
```

```
<frame_start>
```

```
RANGE BETWEEN CURRENT ROW AND offset PRECEDING
```

```
ROWS BETWEEN 7 PRECEDING AND 8 PRECEDING
```

The FROM FIRST or FROM LAST options are only valid for the nth_value function. They specify whether the n-th value is counted from the first row or the last row in the frame. The default is FROM FIRST.

```
FROM FIRST
```

```
FROM LAST
```

```
nth_value
```

```
FROM FIRST
```

For some window functions, NULL values within a window frame can cause undesired results. A common example is a table imported from a spreadsheet where values aren't repeated for rows of the same group. For example:

```
NULL
```

```
1row_no | country | region | amount
2 -------+---------+--------+-------
3      1 | USA     | North  |   1000
4      2 | NULL    | East   |   1200
5      3 | NULL    | West   |   3000
6      4 | NULL    | South  |   2600
7      5 | Germany | North  |   1800
8      6 | NULL    | East   |   2700
9      7 | NULL    | West   |   1100
10      8 | NULL    | South  |   2100
```

In this example, the country value only occurs in the first value of each group, but a query with a filter by region can return all rows with NULL in the country column. You can fix this with the IGNORE NULLS clause and the last_value function:

```
country
```

```
region
```

```
NULL
```

```
country
```

```
IGNORE NULLS
```

```
last_value
```

```
1SELECT last_value(country) IGNORE NULLS OVER (ORDER BY row_no)
2   region,
3   amount
4FROM regions
```

This query produces the result:

```
1country | region | amount
2--------+--------+-------
3USA     | North  |   1000
4USA     | East   |   1200
5USA     | West   |   3000
6USA     | South  |   2600
7Germany | North  |   1800
8Germany | East   |   2700
9Germany | West   |   1100
10Germany | South  |   2100
```

IGNORE NULLS is only supported for the last_value function. The clause RESPECT NULLS doesn't ignore NULL values and is the default behavior.

```
IGNORE NULLS
```

```
last_value
```

```
RESPECT NULLS
```

```
NULL
```

In Data 360 SQL the IGNORE NULLS option is only supported for the last_value function.

```
IGNORE NULLS
```

```
last_value
```

Note

To caing parameter-less aggregate functions as window functions, use syntaxes with *. For example count(*) OVER (PARTITION BY x ORDER BY y).Window-specific functions don’t support DISTINCT or ORDER BY within the function argument list.

```
*.
```

```
count(*) OVER (PARTITION BY x ORDER BY y)
```

```
DISTINCT
```

```
ORDER BY
```

Window function calls can only be in the SELECT list and the ORDER BY clause of the query.

```
SELECT
```

```
ORDER BY
```



---

# Set Returning Functions | Data 360 SQL Reference | Data 360 Query Guide

_URL: https://developer.salesforce.com/docs/data/data-cloud-query-guide/references/dc-sql-reference/set-returning-functions.html_

## Set Returning Functions

Applies to: ✅ Data 360 SQL ✅ Tableau Hyper API

This section describes functions that return multiple rows. Set returning functions can be called in the FROM clause of a SQL query:

```
1SELECT * FROM unnest(ARRAY['Mon','Tue','Wed','Thu','Fri']) AS days(day)
```

- unnest - Expands the elements of an array.

```
unnest
```

- external - Reads data stored in an external file format from one or multiple external locations.

```
external
```

- generate_series (Numerical) - Generates a series of numerical values from start to stop with a given step size.

```
generate_series
```

- generate_series (Time-based) - Generates a series of date or timestamp values from start to stop with a given interval step size.

```
generate_series
```

In SQL, all row sets are unordered by default. Due to implementation details such as parallelization, rows might be processed and output in any order.

Sometimes, the order of rows generated by a set returning function is important, though. In such cases, the WITH ORDINALITY clause can be used to add an additional output column, assigning a sequential number to the generated rows:

```
WITH ORDINALITY
```

```
1SELECT *
2FROM unnest(ARRAY['Mon','Tue','Wed','Thu','Fri']) WITH ORDINALITY AS days(day, ordinality)
3ORDER BY ordinality
```

Results:

day

ordinality

Mon

1

Tue

2

Wed

3

Thu

4

Fri

5

The WITH ORDINALITY clause is supported by most, but not all, set returning functions.

```
WITH ORDINALITY
```

- SELECT

- FROM Clause



---

# SQL Commands for Tableau Hyper API | Data 360 SQL Reference | Data 360 Query Guide

_URL: https://developer.salesforce.com/docs/data/data-cloud-query-guide/references/dc-sql-reference/tableau-sql-commands-context.html_

## SQL Commands for Tableau Hyper API

Applies to: ❌ Data 360 SQL ✅ Tableau Hyper API

Check out these SQL commands that perform write operations.

For more information about write operations in Tableau Hyper API, see Execute SQL Commands.

The Hyper API SQL commands can be grouped into these categories:

- Data modification commands: Using INSERT, UPDATE, DELETE, and TRUNCATE, one can change the data stored inside a table. Furthermore, COPY FROM copies data directly from an external file (e.g., a CSV or Parquet file) into a table. When combined with SELECT queries, one can also use those commands to, e.g., materialize the results of complex analytical computations.

- Data definition commands: Data Cloud SQL supports commands to create databases, tables and modify ("alter") and delete ("drop") them.

- Support statements for prepared statements: PREPARE can pre-compile a query, including placeholder parameters. A prepared query can later on be executed through EXECUTE, giving specific values for the parameters. DEALLOCATE deallocates a given query.
