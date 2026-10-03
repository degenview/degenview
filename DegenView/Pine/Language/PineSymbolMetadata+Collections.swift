import Foundation

extension PineSymbolMetadata {
    /// `array.*`, `map.*` and `matrix.*`.
    static let collectionEntries = [arrayEntries, mapEntries, matrixEntries].joined(separator: "\n")

    private static let arrayEntries = """
        array.new_float(size: series int = 0, initial_value: series float = na) -> array<float> :: A float array.
        array.new_int(size: series int = 0, initial_value: series int = na) -> array<int> :: An int array.
        array.new_bool(size: series int = 0, initial_value: series bool = na) -> array<bool> :: A bool array.
        array.new_string(size: series int = 0, initial_value: series string = na) -> array<string> :: A string array.
        array.new_color(size: series int = 0, initial_value: series color = na) -> array<color> :: A color array.
        array.new_line(size: series int = 0, initial_value: series line = na) -> array<line> :: A line array.
        array.new_label(size: series int = 0, initial_value: series label = na) -> array<label> :: A label array.
        array.new_box(size: series int = 0, initial_value: series box = na) -> array<box> :: A box array.
        array.new_table(size: series int = 0, initial_value: series table = na) -> array<table> :: A table array.
        array.new_linefill(size: series int = 0, initial_value: series linefill = na) -> array<linefill> :: A line fill array.
        array.from(arg0: series any) -> array<any> :: An array of the arguments.
        array.size(id: array<any>) -> series int :: Number of elements.
        array.get(id: array<any>, index: series int) -> series any :: Element at an index.
        array.set(id: array<any>, index: series int, value: series any) -> void :: Replaces an element.
        array.push(id: array<any>, value: series any) -> void :: Appends an element.
        array.pop(id: array<any>) -> series any :: Removes and returns the last element.
        array.shift(id: array<any>) -> series any :: Removes and returns the first element.
        array.unshift(id: array<any>, value: series any) -> void :: Prepends an element.
        array.insert(id: array<any>, index: series int, value: series any) -> void :: Inserts an element at an index.
        array.remove(id: array<any>, index: series int) -> series any :: Removes and returns the element at an index.
        array.clear(id: array<any>) -> void :: Removes every element.
        array.first(id: array<any>) -> series any :: The first element.
        array.last(id: array<any>) -> series any :: The last element.
        array.includes(id: array<any>, value: series any) -> series bool :: True when the value is in the array.
        array.indexof(id: array<any>, value: series any) -> series int :: Index of the value, or -1.
        array.reverse(id: array<any>) -> void :: Reverses the array in place.
        array.sort(id: array<any>, order: series string = order.ascending) -> void :: Sorts the array in place.
        array.sort_indices(id: array<any>, order: series string = order.ascending) -> array<int> :: Indices that would sort the array.
        array.slice(id: array<any>, index_from: series int, index_to: series int) -> array<any> :: A copy of part of the array.
        array.copy(id: array<any>) -> array<any> :: A copy of the array.
        array.concat(id: array<any>, other: array<any>) -> array<any> :: Appends another array.
        array.join(id: array<any>, separator: series string = ",") -> series string :: Joins the elements into text.
        array.sum(id: array<float>) -> series float :: Sum of the elements.
        array.avg(id: array<float>) -> series float :: Mean of the elements.
        array.min(id: array<float>) -> series float :: Smallest element.
        array.max(id: array<float>) -> series float :: Largest element.
        array.median(id: array<float>) -> series float :: Median of the elements.
        array.mode(id: array<float>) -> series float :: Most frequent element.
        array.range(id: array<float>) -> series float :: Largest minus smallest element.
        array.stdev(id: array<float>, biased: series bool = true) -> series float :: Standard deviation.
        array.variance(id: array<float>, biased: series bool = true) -> series float :: Variance.
        array.covariance(id: array<float>, id2: array<float>, biased: series bool = true) -> series float :: Covariance of two arrays.
        array.percentile_nearest_rank(id: array<float>, percentage: series float) -> series float :: Percentile by nearest rank.
        array.percentile_linear_interpolation(id: array<float>, percentage: series float) -> series float :: Percentile by linear interpolation.
        """

    private static let mapEntries = """
        map.new() -> map<any, any> :: An empty map.
        map.put(id: map<any, any>, key: series any, value: series any) -> series any :: Stores a value under a key.
        map.put_all(id: map<any, any>, id2: map<any, any>) -> void :: Copies every entry of another map.
        map.get(id: map<any, any>, key: series any) -> series any :: The value under a key, or na.
        map.contains(id: map<any, any>, key: series any) -> series bool :: True when the key is present.
        map.remove(id: map<any, any>, key: series any) -> series any :: Removes a key and returns its value.
        map.size(id: map<any, any>) -> series int :: Number of entries.
        map.clear(id: map<any, any>) -> void :: Removes every entry.
        map.keys(id: map<any, any>) -> array<any> :: The keys.
        map.values(id: map<any, any>) -> array<any> :: The values.
        map.copy(id: map<any, any>) -> map<any, any> :: A copy of the map.
        """

    private static let matrixEntries = """
        matrix.new(rows: series int = 0, columns: series int = 0, initial_value: series any = na) -> matrix<any> :: A matrix.
        matrix.get(id: matrix<any>, row: series int, column: series int) -> series any :: Element at a row and column.
        matrix.set(id: matrix<any>, row: series int, column: series int, value: series any) -> void :: Replaces an element.
        matrix.row(id: matrix<any>, row: series int) -> array<any> :: A row as an array.
        matrix.col(id: matrix<any>, column: series int) -> array<any> :: A column as an array.
        matrix.add_row(id: matrix<any>, row: series int = na, array_id: array<any> = na) -> void :: Inserts a row.
        matrix.add_col(id: matrix<any>, column: series int = na, array_id: array<any> = na) -> void :: Inserts a column.
        matrix.remove_row(id: matrix<any>, row: series int = na) -> array<any> :: Removes a row.
        matrix.remove_col(id: matrix<any>, column: series int = na) -> array<any> :: Removes a column.
        matrix.fill(id: matrix<any>, value: series any) -> void :: Sets every element.
        matrix.copy(id: matrix<any>) -> matrix<any> :: A copy of the matrix.
        matrix.transpose(id: matrix<any>) -> matrix<any> :: The transposed matrix.
        matrix.rows(id: matrix<any>) -> series int :: Number of rows.
        matrix.columns(id: matrix<any>) -> series int :: Number of columns.
        matrix.elements_count(id: matrix<any>) -> series int :: Number of elements.
        matrix.avg(id: matrix<float>) -> series float :: Mean of the elements.
        matrix.min(id: matrix<float>) -> series float :: Smallest element.
        matrix.max(id: matrix<float>) -> series float :: Largest element.
        """
}
